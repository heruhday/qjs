use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use built_ins::BuiltinHost;
use codegen::{CompiledBytecode, compile_source};
use disasm::disassemble_compiled_clean;
use value::is_undefined;
use vm::{VM, optimization};

const ACC: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Run { path: PathBuf, optimize: bool },
    Disasm { path: PathBuf, optimize: bool },
    Repl { optimize: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub console_output: Vec<String>,
    pub value_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplOutcome {
    pub new_console_output: Vec<String>,
    pub value_display: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReplSession {
    history: Vec<String>,
    previous_console_output: Vec<String>,
    optimize: bool,
}

impl ReplSession {
    pub fn new(optimize: bool) -> Self {
        Self {
            history: Vec::new(),
            previous_console_output: Vec::new(),
            optimize,
        }
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn reset(&mut self) {
        self.history.clear();
        self.previous_console_output.clear();
    }

    pub fn eval(&mut self, snippet: &str) -> Result<ReplOutcome, String> {
        let snippet = snippet.trim_end_matches(['\r', '\n']);
        let candidate_source = self.combined_source_with(snippet);
        let result = execute_source(&candidate_source, self.optimize)?;
        let prefix_len = common_prefix_len(&self.previous_console_output, &result.console_output);
        let new_console_output = result.console_output[prefix_len..].to_vec();

        self.history.push(snippet.to_owned());
        self.previous_console_output = result.console_output;

        Ok(ReplOutcome {
            new_console_output,
            value_display: result.value_display,
        })
    }

    fn combined_source_with(&self, snippet: &str) -> String {
        if self.history.is_empty() {
            snippet.to_owned()
        } else {
            format!("{}\n{}", self.history.join("\n"), snippet)
        }
    }
}

pub fn parse_cli_args<I>(args: I) -> Result<CliCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut optimize = false;
    let mut positionals = Vec::new();

    for arg in args.into_iter().skip(1) {
        if arg.is_empty() {
            continue;
        }

        match arg.as_str() {
            "-h" | "--help" | "help" => return Ok(CliCommand::Help),
            "-O" | "--optimize" => optimize = true,
            "--release" => continue,
            _ if arg.starts_with('-') => {
                return Err(format!(
                    "unknown option `{arg}`\n\n{}",
                    help_text("qjs-repl")
                ));
            }
            _ => positionals.push(arg),
        }
    }

    match positionals.as_slice() {
        [] => Ok(CliCommand::Repl { optimize: true }),
        [command] if command == "repl" => Ok(CliCommand::Repl { optimize }),
        [path] => Ok(CliCommand::Run {
            path: PathBuf::from(path),
            optimize,
        }),
        [command, path] if command == "run" || command == "load" => Ok(CliCommand::Run {
            path: PathBuf::from(path),
            optimize,
        }),
        [command, path] if command == "disasm" || command == "disassemble" => {
            Ok(CliCommand::Disasm {
                path: PathBuf::from(path),
                optimize,
            })
        }
        _ => Err(format!("invalid arguments\n\n{}", help_text("qjs-repl"))),
    }
}

pub fn help_text(program: &str) -> String {
    format!(
        "\
Usage:
  {program} [--optimize] [repl]
  {program} [--optimize] <file.js>
  {program} [--optimize] run <file.js>
  {program} [--optimize] load <file.js>
  {program} [--optimize] disasm <file.js>

Modes:
  repl            Start the interactive REPL.
  run/load        Compile and execute a JavaScript file.
  disasm          Compile a JavaScript file and print bytecode assembly.

Flags:
  -O, --optimize  Run the SSA/direct optimization pipeline before execution.
  -h, --help      Show this help text.

REPL commands:
  .help           Show REPL help.
  .load <file>    Load a file into the current session.
  .disasm <file>  Disassemble a file from inside the REPL.
  .history        Print accepted session input.
  .reset          Clear the current session.
  .exit, .quit    Leave the REPL.

Notes:
  Running with no arguments starts `repl` with `--optimize`.
  The REPL is source-accumulating: each accepted input is appended to the
  session, recompiled, and re-executed so previous bindings remain available."
    )
}

pub fn read_source_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

pub fn compile_program(source: &str, optimize: bool) -> Result<CompiledBytecode, String> {
    let compiled = compile_source(source).map_err(|error| error.to_string())?;
    if optimize && compiled.string_constants.is_empty() {
        Ok(optimization::optimize_compiled(compiled))
    } else {
        Ok(compiled)
    }
}

pub fn execute_source(source: &str, optimize: bool) -> Result<ExecutionResult, String> {
    let compiled = compile_program(source, optimize)?;
    let mut vm = VM::from_compiled(compiled, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    let result = vm.frame.regs[ACC];
    let value_display = (!is_undefined(result)).then(|| vm.display_string(result));
    let console_output = std::mem::take(&mut vm.console_output);

    Ok(ExecutionResult {
        console_output,
        value_display,
    })
}

pub fn execute_file(path: &Path, optimize: bool) -> Result<ExecutionResult, String> {
    let source = read_source_file(path)?;
    execute_source(&source, optimize)
}

pub fn disassemble_source(source: &str, optimize: bool) -> Result<Vec<String>, String> {
    let compiled = compile_program(source, optimize)?;
    Ok(disassemble_compiled_clean(&compiled))
}

pub fn disassemble_file(path: &Path, optimize: bool) -> Result<Vec<String>, String> {
    let source = read_source_file(path)?;
    disassemble_source(&source, optimize)
}

pub fn run_command(command: CliCommand) -> Result<(), String> {
    match command {
        CliCommand::Help => {
            println!("{}", help_text("qjs-repl"));
            Ok(())
        }
        CliCommand::Run { path, optimize } => {
            let result = execute_file(&path, optimize)?;
            print_execution_result(&result);
            Ok(())
        }
        CliCommand::Disasm { path, optimize } => {
            for line in disassemble_file(&path, optimize)? {
                println!("{line}");
            }
            Ok(())
        }
        CliCommand::Repl { optimize } => run_repl(optimize),
    }
}

pub fn run_repl(optimize: bool) -> Result<(), String> {
    println!("qjs REPL");
    println!("Type .help for commands. Type .exit to quit.");
    println!(
        "Session mode: source-accumulating{}\n",
        if optimize { " (optimized)" } else { "" }
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout();
    let mut session = ReplSession::new(optimize);

    loop {
        print!("qjs> ");
        stdout
            .flush()
            .map_err(|error| format!("failed to flush stdout: {error}"))?;

        let Some(line) = lines.next() else {
            println!();
            break;
        };
        let line = line.map_err(|error| format!("failed to read stdin: {error}"))?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        match trimmed {
            ".exit" | ".quit" => break,
            ".help" => {
                println!("{}", help_text("qjs-repl"));
            }
            ".history" => print_history(&session),
            ".reset" => {
                session.reset();
                println!("session cleared");
            }
            _ if trimmed.starts_with(".load ") => {
                let path = trimmed.trim_start_matches(".load ").trim();
                if path.is_empty() {
                    eprintln!("expected a file path after `.load`");
                    continue;
                }

                match read_source_file(Path::new(path)).and_then(|source| session.eval(&source)) {
                    Ok(outcome) => print_repl_outcome(&outcome),
                    Err(error) => eprintln!("{error}"),
                }
            }
            _ if trimmed.starts_with(".disasm ") => {
                let path = trimmed.trim_start_matches(".disasm ").trim();
                if path.is_empty() {
                    eprintln!("expected a file path after `.disasm`");
                    continue;
                }

                match disassemble_file(Path::new(path), optimize) {
                    Ok(lines) => {
                        for line in lines {
                            println!("{line}");
                        }
                    }
                    Err(error) => eprintln!("{error}"),
                }
            }
            _ => match session.eval(&line) {
                Ok(outcome) => print_repl_outcome(&outcome),
                Err(error) => eprintln!("{error}"),
            },
        }
    }

    Ok(())
}

fn print_execution_result(result: &ExecutionResult) {
    for line in &result.console_output {
        println!("{line}");
    }

    if let Some(value_display) = &result.value_display {
        println!("=> {value_display}");
    }
}

fn print_repl_outcome(outcome: &ReplOutcome) {
    for line in &outcome.new_console_output {
        println!("{line}");
    }

    if let Some(value_display) = &outcome.value_display {
        println!("=> {value_display}");
    }
}

fn print_history(session: &ReplSession) {
    if session.history().is_empty() {
        println!("history is empty");
        return;
    }

    for (index, snippet) in session.history().iter().enumerate() {
        println!("[{}]", index + 1);
        println!("{snippet}");
    }
}

fn common_prefix_len(lhs: &[String], rhs: &[String]) -> usize {
    lhs.iter()
        .zip(rhs)
        .take_while(|(left, right)| left == right)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_repl_mode() {
        let command = parse_cli_args(vec!["qjs-repl".to_owned()]).expect("command");
        assert_eq!(command, CliCommand::Repl { optimize: true });
    }

    #[test]
    fn ignores_empty_args_and_stray_release_flag() {
        let command = parse_cli_args(vec![
            "qjs-repl".to_owned(),
            String::new(),
            "--release".to_owned(),
        ])
        .expect("command");

        assert_eq!(command, CliCommand::Repl { optimize: true });
    }

    #[test]
    fn parses_run_path_without_subcommand() {
        let command =
            parse_cli_args(vec!["qjs-repl".to_owned(), "test.js".to_owned()]).expect("command");
        assert_eq!(
            command,
            CliCommand::Run {
                path: PathBuf::from("test.js"),
                optimize: false,
            }
        );
    }

    #[test]
    fn parses_disasm_with_optimization() {
        let command = parse_cli_args(vec![
            "qjs-repl".to_owned(),
            "--optimize".to_owned(),
            "disasm".to_owned(),
            "bench.js".to_owned(),
        ])
        .expect("command");

        assert_eq!(
            command,
            CliCommand::Disasm {
                path: PathBuf::from("bench.js"),
                optimize: true,
            }
        );
    }

    #[test]
    fn executes_simple_expression() {
        let result = execute_source("1 + 2;", false).expect("execution");
        assert_eq!(result.console_output, Vec::<String>::new());
        assert_eq!(result.value_display.as_deref(), Some("3"));
    }

    #[test]
    fn executes_optimized_string_concatenation() {
        let result = execute_source("'a' + 'b';", true).expect("execution");
        assert_eq!(result.console_output, Vec::<String>::new());
        assert_eq!(result.value_display.as_deref(), Some("ab"));
    }

    #[test]
    fn executes_optimized_template_literal_with_binding() {
        let result =
            execute_source(r#"let name = 'xx'; `hello, ${name}`"#, true).expect("execution");
        assert_eq!(result.console_output, Vec::<String>::new());
        assert_eq!(result.value_display.as_deref(), Some("hello, xx"));
    }

    #[test]
    fn repl_session_keeps_prior_bindings() {
        let mut session = ReplSession::new(false);

        let first = session.eval("let x = 40;").expect("first eval");
        assert_eq!(first.new_console_output, Vec::<String>::new());
        assert_eq!(first.value_display, None);

        let second = session.eval("console.log(x);").expect("second eval");
        assert_eq!(second.new_console_output, vec!["40"]);
        assert_eq!(second.value_display, None);

        let third = session.eval("x + 2;").expect("third eval");
        assert_eq!(third.new_console_output, Vec::<String>::new());
        assert_eq!(third.value_display.as_deref(), Some("42"));
    }

    #[test]
    fn disassembles_source() {
        let lines = disassemble_source("1 + 2;", false).expect("disassembly");
        assert!(!lines.is_empty());
        assert_eq!(lines.last().map(String::as_str), Some("ret"));
    }
}
