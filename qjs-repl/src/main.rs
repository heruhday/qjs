fn main() {
    match qjs_repl::parse_cli_args(std::env::args()).and_then(qjs_repl::run_command) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
