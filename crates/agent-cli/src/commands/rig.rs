use agent_domain::RigSpec;
use agent_runtime::KernelOperator;

use crate::cli::RigAction;

pub async fn handle(action: RigAction) -> anyhow::Result<()> {
    match action {
        RigAction::Up { spec, workspace } => {
            let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
            })?;
            let rig_spec = RigSpec::from_yaml_str(&yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML RigSpec: {}", e))?;

            println!(
                "🏗️  Booting Rig: {} ({})",
                rig_spec.rig.name, rig_spec.rig.id
            );
            println!(
                "👑 Kernel Operator: {} (Role: {})",
                rig_spec.operator.agent_id,
                rig_spec
                    .operator
                    .role
                    .as_deref()
                    .unwrap_or("Lead Coordinator")
            );

            let op = KernelOperator::new(rig_spec, &workspace);
            let booted = op.boot_seats().map_err(|e| anyhow::anyhow!(e))?;

            println!("\n👥 Booted Persistent Seats ({}):", booted.len());
            for seat in op.inspect_seats() {
                let harness_info = if seat.harness == "tmux" {
                    format!("tmux session: {}", seat.session_name.unwrap_or_default())
                } else {
                    "in-memory process".to_string()
                };
                println!(
                    "   • Seat [{}] - {} | Role: {} | Engine: {} | [{}]",
                    seat.seat_id, seat.name, seat.role, seat.engine, harness_info
                );
            }
            println!(
                "\n✨ Rig is up and ready for dispatch. Use `agent-ctl rig run --goal \"...\"`"
            );
        }
        RigAction::Down { spec } => {
            let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
            })?;
            let rig_spec = RigSpec::from_yaml_str(&yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

            let op = KernelOperator::new(rig_spec, ".");
            op.shutdown_seats().map_err(|e| anyhow::anyhow!(e))?;
            println!(
                "🛑 Successfully shutdown all seats for Rig '{}'",
                spec.display()
            );
        }
        RigAction::Status { spec } => {
            let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
            })?;
            let rig_spec = RigSpec::from_yaml_str(&yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

            println!("📊 Rig Status: {} ({})", rig_spec.rig.name, rig_spec.rig.id);
            println!("👑 Operator: {}", rig_spec.operator.agent_id);
            let op = KernelOperator::new(rig_spec, ".");
            let seats = op.inspect_seats();

            println!("\n🪑 Seat Roster ({} seats):", seats.len());
            for seat in seats {
                let status_str = if seat.is_active {
                    "ACTIVE 🟢"
                } else {
                    "STOPPED ⚪"
                };
                println!(
                    "   • [{}] {} ({}) - Engine: {} | Harness: {} -> {}",
                    seat.seat_id, seat.name, seat.role, seat.engine, seat.harness, status_str
                );
            }
        }
        RigAction::Run {
            spec,
            goal,
            workspace,
            engine,
        } => {
            let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
            })?;
            let rig_spec = RigSpec::from_yaml_str(&yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML: {}", e))?;

            println!(
                "🎯 Giao mục tiêu cho Rig '{}': \"{}\"",
                rig_spec.rig.name, goal
            );
            if let Some(ref eng) = engine {
                println!("⚙️  Configured Engine Override: {}", eng);
            }

            let op = KernelOperator::new(rig_spec, &workspace);
            let summary = op
                .execute_goal(&goal, engine.as_deref())
                .await
                .map_err(|e| anyhow::anyhow!(e))?;

            println!("\n📋 Báo Cáo Thực Thi Đội Ngũ (Execution Summary):");
            println!(
                "   • Rig: {} | Status: {}",
                summary.rig_name, summary.status
            );
            println!(
                "   • Bắt đầu: {} | Hoàn thành: {}",
                summary.started_at, summary.completed_at
            );
            println!("\n🧩 Phân Bổ Tác Vụ Cho Các Seats:");
            for (idx, assign) in summary.assignments.iter().enumerate() {
                println!(
                    "   [Tác vụ #{}] Seat: {} -> Trạng thái: {}",
                    idx + 1,
                    assign.seat_id,
                    assign.status
                );
                if let Some(ref out) = assign.output {
                    let preview: String = out.lines().take(3).collect::<Vec<_>>().join(" ");
                    println!("      Kết quả: {}", preview);
                }
            }
            if !summary.channel_dispatches.is_empty() {
                println!("\n🔀 Điều Phối Kênh Nội Bộ (Rig Channels Pipeline):");
                for (idx, ch) in summary.channel_dispatches.iter().enumerate() {
                    println!(
                        "   [Chuyển tiếp #{}] Đích: {} -> Trạng thái: {}",
                        idx + 1,
                        ch.seat_id,
                        ch.status
                    );
                    if let Some(ref out) = ch.output {
                        println!("      Nội dung: {}", out);
                    }
                }
            }
            println!("\n📢 Tổng kết từ Kernel Operator:");
            println!("   {}", summary.operator_synthesis);
        }
        RigAction::Snapshot {
            spec,
            workspace,
            out,
        } => {
            let yaml = tokio::fs::read_to_string(&spec).await.map_err(|e| {
                anyhow::anyhow!("Không thể đọc file spec '{}': {}", spec.display(), e)
            })?;
            let rig_spec = RigSpec::from_yaml_str(&yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp YAML RigSpec: {}", e))?;

            println!(
                "📸 Tạo Snapshot cho Rig: {} ({})",
                rig_spec.rig.name, rig_spec.rig.id
            );
            let op = KernelOperator::new(rig_spec, &workspace);
            let snapshot = op.create_snapshot().map_err(|e| anyhow::anyhow!(e))?;
            let saved_path = op
                .save_snapshot(&snapshot, out.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;

            println!("✅ Snapshot đã được lưu thành công tại: {}", saved_path);
            println!("   • Snapshot ID: {}", snapshot.snapshot_id);
            println!("   • Số lượng seats: {}", snapshot.seats.len());
            for seat in &snapshot.seats {
                let active_str = if seat.is_active {
                    "Đang hoạt động"
                } else {
                    "Đã dừng"
                };
                println!(
                    "     - Seat: {} (Harness: {}, {})",
                    seat.seat_id, seat.harness, active_str
                );
            }
        }
        RigAction::Restore {
            snapshot,
            workspace,
        } => {
            let snap = KernelOperator::load_snapshot(&snapshot.to_string_lossy())
                .map_err(|e| anyhow::anyhow!(e))?;
            let rig_spec = RigSpec::from_yaml_str(&snap.spec_yaml)
                .map_err(|e| anyhow::anyhow!("Lỗi cú pháp RigSpec trong snapshot: {}", e))?;

            println!(
                "🔄 Khôi phục Rig '{}' từ snapshot: {}",
                snap.rig_name, snap.snapshot_id
            );
            let op = KernelOperator::new(rig_spec, &workspace);
            op.restore_snapshot(&snap).map_err(|e| anyhow::anyhow!(e))?;

            println!(
                "✅ Đã khôi phục thành công các seats của Rig: {}",
                snap.rig_id
            );
        }
    }

    Ok(())
}

