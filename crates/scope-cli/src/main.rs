use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "sx")]
#[command(about = "Scope VCS command-line prototype")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("Scope CLI has no repository commands yet.");
}
