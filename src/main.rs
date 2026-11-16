use clap::{Parser, Subcommand};

use andrei_os::{config::Config, db, sync};

#[derive(Parser)]
#[command(
    name = "andrei-os",
    about = "AndreiOS: second brain synced from Apple Notes, Reminders and Calendar"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the web app (default). Also syncs in the background.
    Serve,
    /// Sync once from Apple Notes, Reminders and Calendar, then exit.
    Sync,
    /// Transcribe Notes audio recordings with whisper.cpp.
    Transcribe {
        /// Also replace Apple's own transcripts (e.g. for Romanian recordings).
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "andrei_os=info,tower_http=warn".into()),
        )
        .init();
    let cfg = Config::from_env()?;
    cfg.ensure_dirs()?;
    let pool = db::open(&cfg.db_path()).await?;

    match Cli::parse().cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => andrei_os::web::serve(cfg, pool).await?,
        Cmd::Sync => {
            let r = sync::run(&cfg, &pool).await;
            for l in &r.lines {
                println!("{l}");
            }
            for w in &r.warnings {
                println!("warning: {w}");
            }
            for e in &r.errors {
                eprintln!("error: {e}");
            }
            if !r.ok {
                std::process::exit(1);
            }
        }
        Cmd::Transcribe { force } => {
            let r = sync::audio::transcribe(&cfg, &pool, i64::MAX, force).await?;
            println!("{} of {} recordings transcribed", r.transcribed, r.pending);
            for w in &r.warnings {
                println!("warning: {w}");
            }
        }
    }
    Ok(())
}
