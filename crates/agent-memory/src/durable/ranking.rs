use crate::durable::storage::{extract_query_tokens, StructuredLesson};
use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// Tham số cấu hình chuẩn cho thuật toán xếp hạng Okapi BM25.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bm25Params {
    /// Hệ số bão hòa tần suất từ khóa (thường từ 1.2 đến 2.0).
    pub k1: f64,
    /// Hệ số chuẩn hóa độ dài tài liệu (0.0 = tắt, 0.75 = chuẩn).
    pub b: f64,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.5, b: 0.75 }
    }
}

/// Chi tiết cấu thành điểm số Hybrid Semantic Scoring của một bản ghi bộ nhớ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HybridScoreBreakdown {
    pub bm25_score: f64,
    pub coverage_ratio: f64,
    pub confidence_weight: f64,
    pub recency_weight: f64,
    pub final_score: f64,
}

const DEFAULT_BASELINE_DOC_LEN: f64 = 16.0;

/// Tách văn bản tài liệu thành danh sách token phục vụ tính toán BM25.
pub fn tokenize_document(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|t| t.trim_matches(|c: char| c == '_' || c == '-').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Kiểm tra một token trong tài liệu có khớp với token truy vấn hay không
/// (hỗ trợ khớp chính xác, khớp sub-token phân tách bởi `_`/`-`, và biến thể số nhiều/tiền tố ngắn).
pub fn tokens_match(doc_token: &str, query_token: &str) -> bool {
    if doc_token.is_empty() || query_token.is_empty() {
        return false;
    }
    if doc_token == query_token {
        return true;
    }
    if prefix_variant_match(doc_token, query_token) {
        return true;
    }
    if doc_token.contains('_') || doc_token.contains('-') {
        for sub in doc_token.split(['_', '-']).filter(|s| !s.is_empty()) {
            if sub == query_token || prefix_variant_match(sub, query_token) {
                return true;
            }
        }
    }
    false
}

fn prefix_variant_match(a: &str, b: &str) -> bool {
    (a.len() > b.len() && a.starts_with(b) && a.len() - b.len() <= 2)
        || (b.len() > a.len() && b.starts_with(a) && b.len() - a.len() <= 2)
}

/// Tính chỉ số Inverse Document Frequency (IDF) không âm theo công thức Robertson-Spärck Jones / Lucene.
pub fn compute_idf(doc_freq: usize, total_docs: usize) -> f64 {
    let n = (total_docs.max(1)) as f64;
    let df = (doc_freq.min(total_docs.max(1))) as f64;
    (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
}

/// Tính điểm BM25 cho một từ khóa đơn lẻ dựa trên TF, độ dài tài liệu, độ dài trung bình và IDF.
pub fn compute_bm25_term_score(
    tf: f64,
    doc_len: usize,
    avg_doc_len: f64,
    idf: f64,
    params: Bm25Params,
) -> f64 {
    if tf <= 0.0 || idf <= 0.0 {
        return 0.0;
    }
    let dl = (doc_len.max(1)) as f64;
    let avgdl = avg_doc_len.max(1.0);
    let length_norm = 1.0 - params.b + params.b * (dl / avgdl);
    let numerator = tf * (params.k1 + 1.0);
    let denominator = tf + params.k1 * length_norm;
    if denominator <= 0.0 {
        0.0
    } else {
        idf * (numerator / denominator)
    }
}

/// Tính điểm BM25 tổng hợp của một văn bản tài liệu đối với câu truy vấn trên một tập tài liệu (corpus).
pub fn compute_bm25_score<S: AsRef<str>>(query: &str, doc: &str, corpus: &[S]) -> f64 {
    compute_bm25_score_with_params(query, doc, corpus, Bm25Params::default())
}

/// Tính điểm BM25 với bộ tham số tùy chỉnh `Bm25Params`.
pub fn compute_bm25_score_with_params<S: AsRef<str>>(
    query: &str,
    doc: &str,
    corpus: &[S],
    params: Bm25Params,
) -> f64 {
    let query_tokens = deduplicate_tokens(extract_query_tokens(query));
    if query_tokens.is_empty() {
        return 0.0;
    }
    let doc_tokens = tokenize_document(doc);
    if doc_tokens.is_empty() {
        return 0.0;
    }

    let tokenized_corpus: Vec<Vec<String>> = corpus
        .iter()
        .map(|c| tokenize_document(c.as_ref()))
        .filter(|tokens| !tokens.is_empty())
        .collect();

    compute_bm25_from_tokenized(&query_tokens, &doc_tokens, &tokenized_corpus, params)
}

/// Xếp hạng danh sách tài liệu theo điểm số Okapi BM25 giảm dần.
/// Chỉ trả về các tài liệu có điểm BM25 > 0.0.
pub fn rank_documents_bm25<S: AsRef<str>>(query: &str, documents: &[S]) -> Vec<(String, f64)> {
    let query_tokens = deduplicate_tokens(extract_query_tokens(query));
    if query_tokens.is_empty() || documents.is_empty() {
        return Vec::new();
    }

    let tokenized_corpus: Vec<Vec<String>> = documents
        .iter()
        .map(|d| tokenize_document(d.as_ref()))
        .filter(|t| !t.is_empty())
        .collect();

    let stats = Bm25CorpusStats::new(&query_tokens, &tokenized_corpus);

    let mut scored: Vec<(usize, String, f64)> = documents
        .iter()
        .enumerate()
        .filter_map(|(idx, doc)| {
            let doc_str = doc.as_ref();
            let doc_tokens = tokenize_document(doc_str);
            let score = stats.score_document(&doc_tokens, Bm25Params::default());
            if score > 0.0 {
                Some((idx, doc_str.to_string(), score))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    scored.into_iter().map(|(_, d, s)| (d, s)).collect()
}

fn deduplicate_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::with_capacity(tokens.len());
    for t in tokens {
        if seen.insert(t.clone()) {
            unique.push(t);
        }
    }
    unique
}

struct Bm25CorpusStats<'a> {
    query_tokens: &'a [String],
    avg_doc_len: f64,
    query_idfs: Vec<f64>,
}

impl<'a> Bm25CorpusStats<'a> {
    fn new(query_tokens: &'a [String], tokenized_corpus: &[Vec<String>]) -> Self {
        let (total_docs, avg_doc_len) = if tokenized_corpus.len() > 1 {
            let total_len: usize = tokenized_corpus.iter().map(|d| d.len()).sum();
            let n = tokenized_corpus.len();
            (n, (total_len as f64) / (n as f64))
        } else {
            (1, DEFAULT_BASELINE_DOC_LEN)
        };

        let query_idfs = query_tokens
            .iter()
            .map(|q| {
                let df = if tokenized_corpus.len() > 1 {
                    tokenized_corpus
                        .iter()
                        .filter(|c_doc| c_doc.iter().any(|dt| tokens_match(dt.as_str(), q.as_str())))
                        .count()
                        .max(1)
                } else {
                    1
                };
                compute_idf(df, total_docs)
            })
            .collect();

        Self {
            query_tokens,
            avg_doc_len,
            query_idfs,
        }
    }

    fn score_document(&self, doc_tokens: &[String], params: Bm25Params) -> f64 {
        if self.query_tokens.is_empty() || doc_tokens.is_empty() {
            return 0.0;
        }
        let mut score = 0.0;
        for (q, &idf) in self.query_tokens.iter().zip(self.query_idfs.iter()) {
            let tf = doc_tokens
                .iter()
                .filter(|dt| tokens_match(dt.as_str(), q.as_str()))
                .count() as f64;
            if tf > 0.0 {
                score += compute_bm25_term_score(tf, doc_tokens.len(), self.avg_doc_len, idf, params);
            }
        }
        score
    }
}

fn compute_bm25_from_tokenized(
    query_tokens: &[String],
    doc_tokens: &[String],
    tokenized_corpus: &[Vec<String>],
    params: Bm25Params,
) -> f64 {
    if query_tokens.is_empty() || doc_tokens.is_empty() {
        return 0.0;
    }
    let stats = Bm25CorpusStats::new(query_tokens, tokenized_corpus);
    stats.score_document(doc_tokens, params)
}

/// Tính hệ số suy giảm theo thời gian (Recency / Time-Decay) dựa trên khoảng cách tới `now`.
pub fn compute_recency_weight(timestamp_rfc3339: &str, now: DateTime<Utc>) -> f64 {
    match DateTime::parse_from_rfc3339(timestamp_rfc3339.trim()) {
        Ok(parsed) => {
            let parsed_utc = parsed.with_timezone(&Utc);
            let age_secs = now
                .signed_duration_since(parsed_utc)
                .num_seconds()
                .max(0) as f64;
            let age_days = age_secs / 86_400.0;
            // Suy giảm mượt theo hàm hyperbolic với ngưỡng sàn lịch sử 0.55 để bài học cũ giàu từ khóa hiếm vẫn giữ giá trị
            0.55 + 0.45 / (1.0 + 0.02 * age_days)
        }
        Err(_) => 0.8,
    }
}

fn lesson_combined_text(lesson: &StructuredLesson) -> String {
    let err_part = lesson.trigger_error.as_deref().unwrap_or("");
    let rem_part = lesson.remediation_applied.as_deref().unwrap_or("");
    format!(
        "{} {} {} {}",
        lesson.task_title, err_part, rem_part, lesson.general_insight
    )
}

fn score_lesson_with_stats(
    lesson: &StructuredLesson,
    doc_tokens: &[String],
    stats: &Bm25CorpusStats<'_>,
    now: DateTime<Utc>,
) -> HybridScoreBreakdown {
    let matched_query_count = stats
        .query_tokens
        .iter()
        .filter(|q| doc_tokens.iter().any(|dt| tokens_match(dt.as_str(), q.as_str())))
        .count();

    if matched_query_count == 0 {
        return HybridScoreBreakdown {
            bm25_score: 0.0,
            coverage_ratio: 0.0,
            confidence_weight: 0.0,
            recency_weight: 0.0,
            final_score: 0.0,
        };
    }

    let bm25_score = stats.score_document(doc_tokens, Bm25Params::default());
    let coverage_ratio = (matched_query_count as f64) / (stats.query_tokens.len() as f64);
    let has_remediation = lesson
        .remediation_applied
        .as_ref()
        .is_some_and(|r| !r.trim().is_empty() && !r.trim().eq_ignore_ascii_case("none"));

    let confidence_weight = if lesson.success {
        if has_remediation { 1.25 } else { 1.15 }
    } else if has_remediation {
        0.65
    } else {
        0.55
    };

    let recency_weight = compute_recency_weight(&lesson.timestamp, now);
    let semantic_coverage_factor = 0.25 + 0.75 * coverage_ratio;
    let final_score = bm25_score * semantic_coverage_factor * confidence_weight * recency_weight;

    HybridScoreBreakdown {
        bm25_score,
        coverage_ratio,
        confidence_weight,
        recency_weight,
        final_score,
    }
}

/// Tính điểm Hybrid Semantic Scoring chi tiết cho một `StructuredLesson` trong ngữ cảnh một tập bài học.
pub fn score_lesson_hybrid(
    query: &str,
    lesson: &StructuredLesson,
    corpus: &[StructuredLesson],
    now: DateTime<Utc>,
) -> HybridScoreBreakdown {
    let query_tokens = deduplicate_tokens(extract_query_tokens(query));
    if query_tokens.is_empty() {
        return HybridScoreBreakdown {
            bm25_score: 0.0,
            coverage_ratio: 0.0,
            confidence_weight: 0.0,
            recency_weight: 0.0,
            final_score: 0.0,
        };
    }

    let doc_tokens = tokenize_document(&lesson_combined_text(lesson));
    let tokenized_corpus: Vec<Vec<String>> = corpus
        .iter()
        .map(|l| tokenize_document(&lesson_combined_text(l)))
        .filter(|t| !t.is_empty())
        .collect();
    let stats = Bm25CorpusStats::new(&query_tokens, &tokenized_corpus);

    score_lesson_with_stats(lesson, &doc_tokens, &stats, now)
}

/// Hàm xếp hạng bài học `score_lesson_relevance(query, lesson, now)` kết hợp BM25 term weighting,
/// semantic coverage, confidence score và recency time-decay.
pub fn score_lesson_relevance(
    query: &str,
    lesson: &StructuredLesson,
    now: DateTime<Utc>,
) -> f64 {
    score_lesson_hybrid(query, lesson, &[], now).final_score
}

/// Xếp hạng danh sách `StructuredLesson` theo điểm số Hybrid Semantic Scoring giảm dần.
pub fn rank_structured_lessons(
    query: &str,
    lessons: &[StructuredLesson],
    now: DateTime<Utc>,
) -> Vec<(StructuredLesson, f64)> {
    let query_tokens = deduplicate_tokens(extract_query_tokens(query));
    if query_tokens.is_empty() || lessons.is_empty() {
        return Vec::new();
    }

    let tokenized_lessons: Vec<Vec<String>> = lessons
        .iter()
        .map(|l| tokenize_document(&lesson_combined_text(l)))
        .collect();
    let tokenized_corpus: Vec<Vec<String>> = tokenized_lessons
        .iter()
        .filter(|t| !t.is_empty())
        .cloned()
        .collect();
    let stats = Bm25CorpusStats::new(&query_tokens, &tokenized_corpus);

    let mut scored: Vec<(usize, StructuredLesson, f64)> = lessons
        .iter()
        .zip(tokenized_lessons.iter())
        .enumerate()
        .filter_map(|(idx, (lesson, doc_tokens))| {
            let breakdown = score_lesson_with_stats(lesson, doc_tokens, &stats, now);
            if breakdown.final_score > 0.0 {
                Some((idx, lesson.clone(), breakdown.final_score))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    scored.into_iter().map(|(_, l, s)| (l, s)).collect()
}

fn strip_learned_prefix(block: &str) -> (Option<&str>, &str) {
    let trimmed = block.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- [Learned ") {
        if let Some(end_bracket) = rest.find(']') {
            let ts = rest[..end_bracket].trim();
            let body = rest[end_bracket + 1..].trim_start();
            return (Some(ts), body);
        }
    }
    (None, trimmed)
}

/// Xếp hạng các khối bài học Markdown (`MEMORY.md`) theo BM25 + Hybrid Semantic Scoring.
pub fn rank_lesson_blocks(
    query: &str,
    blocks: &[String],
    now: DateTime<Utc>,
) -> Vec<(String, f64)> {
    let query_tokens = deduplicate_tokens(extract_query_tokens(query));
    if query_tokens.is_empty() || blocks.is_empty() {
        return Vec::new();
    }

    let tokenized_blocks: Vec<Vec<String>> = blocks
        .iter()
        .map(|b| {
            let (_, body) = strip_learned_prefix(b);
            tokenize_document(body)
        })
        .collect();
    let tokenized_corpus: Vec<Vec<String>> = tokenized_blocks
        .iter()
        .filter(|t| !t.is_empty())
        .cloned()
        .collect();
    let stats = Bm25CorpusStats::new(&query_tokens, &tokenized_corpus);

    let mut scored: Vec<(usize, String, f64)> = Vec::new();
    for (idx, (block, doc_tokens)) in blocks.iter().zip(tokenized_blocks.iter()).enumerate() {
        let (ts_opt, body) = strip_learned_prefix(block);
        let matched_count = query_tokens
            .iter()
            .filter(|q| doc_tokens.iter().any(|dt| tokens_match(dt.as_str(), q.as_str())))
            .count();
        if matched_count == 0 {
            continue;
        }

        let bm25 = stats.score_document(doc_tokens, Bm25Params::default());
        let coverage_ratio = (matched_count as f64) / (query_tokens.len() as f64);
        let confidence = if body.contains("[FAILURE]") {
            0.65
        } else if body.contains("[SUCCESS]") {
            1.2
        } else {
            1.0
        };
        let recency = ts_opt
            .map(|ts| compute_recency_weight(ts, now))
            .unwrap_or(0.85);
        let final_score = bm25 * (0.25 + 0.75 * coverage_ratio) * confidence * recency;
        if final_score > 0.0 {
            scored.push((idx, block.clone(), final_score));
        }
    }

    // Ưu tiên điểm cao nhất; nếu bằng điểm thì ưu tiên bài học mới hơn (idx lớn hơn ở cuối file)
    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.0.cmp(&a.0))
    });

    scored.into_iter().map(|(_, b, s)| (b, s)).collect()
}
