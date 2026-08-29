mod cli;
mod gui;

use clap::Parser;

fn main() {
    let cli_args = cli::Cli::parse();
    let result = if cli_args.gui {
        gui::run_gui()
    } else {
        cli::handle_cli(cli_args)
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
