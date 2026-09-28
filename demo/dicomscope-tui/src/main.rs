//! `dicomscope-tui`: see the crate documentation or `--help`.

#![forbid(unsafe_code)]

use clap::Parser;
use dicomscope_tui::cli::{Cli, Command, GpuArgs, ViewArgs};
use dicomscope_tui::{app, commands, window};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match &cli.command {
        None => viewer(&cli.view, cli.gpu),
        Some(Command::View(v)) => viewer(v, cli.gpu),
        Some(Command::Info(a)) => commands::info(a),
        Some(Command::Tags(a)) => commands::tags(a),
        Some(Command::Render(a)) => commands::render(a, cli.gpu),
        Some(Command::Order(a)) => commands::order(a),
        Some(Command::Link(a)) => commands::link(a),
        Some(Command::Fhir(a)) => commands::fhir(a),
        Some(Command::Worklist(a)) => commands::worklist(a),
        Some(Command::Docs(a)) => commands::docs(a),
        Some(Command::Gpu) => commands::gpu_list(cli.gpu),
    };
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("dicomscope-tui: {e}");
            ExitCode::FAILURE
        }
    }
}

fn viewer(args: &ViewArgs, gpu: GpuArgs) -> commands::Outcome {
    if args.window {
        window::run(args, gpu, args.font.as_ref()).map(|()| 0)
    } else {
        app::run(args, gpu).map(|()| 0).map_err(|e| e.to_string())
    }
}
