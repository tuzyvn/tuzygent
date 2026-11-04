use agent_memory::MemoryService;
use clap::{Args, Subcommand};
use std::path::PathBuf;
#[derive(Args)]
pub struct MemoryArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long, default_value = "default")]
    project: String,
    #[arg(long)]
    persona: String,
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    List,
    AdoptLegacy {
        id: String,
    },
    Show {
        id: String,
    },
    Publish {
        id: String,
    },
    Reject {
        id: String,
    },
    Delete {
        id: String,
    },
    Supersede {
        id: String,
    },
    /// Explicitly review a lesson backed by a verified episode
    Activate {
        id: String,
    },
    Propose {
        #[arg(long)]
        episode: String,
        #[arg(long)]
        content: String,
        #[arg(long, default_value = "")]
        conditions: String,
    },
    Search {
        query: String,
    },
    /// Back up and quarantine an old memory file under __legacy__
    Import {
        path: PathBuf,
    },
}
pub fn run(args: MemoryArgs) -> anyhow::Result<()> {
    let m = MemoryService::open(&args.workspace, &args.project, &args.persona)?;
    match args.action {
        Action::AdoptLegacy { id } => println!("{}", m.adopt_legacy(&id)?),
        Action::List => println!("{}", serde_json::to_string_pretty(&m.list()?)?),
        Action::Show { id } => println!("{}", serde_json::to_string_pretty(&m.show(&id)?)?),
        Action::Publish { id } => println!("{}", m.publish(&id)?),
        Action::Reject { id } => m.reject(&id)?,
        Action::Delete { id } => m.delete(&id)?,
        Action::Supersede { id } => m.supersede(&id)?,
        Action::Activate { id } => m.activate(&id)?,
        Action::Propose {
            episode,
            content,
            conditions,
        } => println!("{}", m.propose_lesson(&content, &conditions, &episode)?),
        Action::Search { query } => println!(
            "{}",
            serde_json::to_string_pretty(&m.retrieve("cli-search", &query)?)?
        ),
        Action::Import { path } => println!("{}", m.import_legacy(&path)?),
    }
    Ok(())
}
