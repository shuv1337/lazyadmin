#![forbid(unsafe_code)]

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "lazyadmin-tui",
    version,
    about = "Ratatui interface for lazyadmin"
)]
struct Args {
    /// Print the responsive view-model for a width instead of entering raw terminal mode.
    #[arg(long)]
    view_model_width: Option<u16>,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    if let Some(width) = args.view_model_width {
        let vm = lazyadmin_tui::build_view_model(
            &lazyadmin_core::snapshot::build_empty_snapshot(),
            width,
            false,
            "",
        );
        println!("{}", serde_json::to_string_pretty(&vm)?);
        return Ok(());
    }
    lazyadmin_tui::run_default().await
}
