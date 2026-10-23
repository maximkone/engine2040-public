mod build;
mod cli;
mod manifest;
mod process;
mod project;
mod workspace;

fn main() -> std::process::ExitCode {
    match cli::run(std::env::args_os().skip(1)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(error.exit_code())
        }
    }
}
