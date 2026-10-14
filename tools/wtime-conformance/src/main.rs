mod adapter;
mod cli;
mod corpus;
mod report;
mod script;
mod spectest;
fn main() {
    if let Err(e) = cli::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
