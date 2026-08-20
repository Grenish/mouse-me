mod cli;
mod gui;

use clap::Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli_args = cli::Cli::parse();

    if cli_args.gui {
        gui::run_gui()?;
    } else {
        cli::handle_cli(cli_args)?;
    }

    Ok(())
}
