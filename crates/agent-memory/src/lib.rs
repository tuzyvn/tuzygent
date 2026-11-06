#![deny(ambiguous_glob_reexports)]
#![deny(clippy::clone_on_copy)]
#![deny(clippy::cloned_instead_of_copied)]
#![deny(suspicious_double_ref_op)]

pub mod context_builder;
pub mod durable;
pub mod persona;
pub mod scoped_scratchpad;
pub mod scratchpad;
pub mod skills;

pub use context_builder::*;
pub use durable::*;
pub use persona::*;
pub use scoped_scratchpad::*;
pub use scratchpad::*;
pub use skills::{
    parse_frontmatter, scan_and_validate_skills, scan_available_skills, validate_frontmatter,
    validate_skill_file, SkillFrontmatter, SkillMetadata, SkillRegistry, SkillValidationError,
    FRONTMATTER_RE,
};

pub mod persona_memory;
pub use persona_memory::{MemoryRecord, MemoryService, RetrievedContext};
