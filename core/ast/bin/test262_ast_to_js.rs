use std::{
    collections::BTreeMap,
    env,
    fmt::Write as _,
    fs,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    process,
};

use ::ast::{ParseOptions, parse_with_options, program_to_js};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expectation {
    ParsePass,
    ParseFail,
}

#[derive(Debug)]
struct Metadata {
    expectation: Expectation,
    is_module: bool,
    is_strict: bool,
}

#[derive(Debug)]
struct FailureRecord {
    path: PathBuf,
    stage: &'static str,
    message: String,
}

#[derive(Debug, Default)]
struct Summary {
    total: usize,
    passed: usize,
    expected_parse_fail: usize,
    module_tests: usize,
    negative_cases_matched: usize,
    positive_cases_matched: usize,
    unexpected_parse_successes: Vec<PathBuf>,
    failures: Vec<FailureRecord>,
}

enum Mode {
    Summary { root: PathBuf, failure_limit: usize },
    Jsonl { root: PathBuf },
}

fn main() {
    let status = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .and_then(|handle| {
            handle
                .join()
                .map_err(|_| std::io::Error::other("worker thread panicked"))
        });

    match status {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("failed to run checker: {error}");
            process::exit(2);
        }
    }
}

fn run() -> i32 {
    let mut args = env::args().skip(1);
    let mode = if matches!(args.next().as_deref(), Some("--jsonl")) {
        Mode::Jsonl {
            root: args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_test262_root),
        }
    } else {
        let mut args = env::args().skip(1);
        let root = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(default_test262_root);
        let failure_limit = args
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(20);
        Mode::Summary {
            root,
            failure_limit,
        }
    };

    match mode {
        Mode::Summary {
            root,
            failure_limit,
        } => run_summary(root, failure_limit),
        Mode::Jsonl { root } => run_jsonl(root),
    }
}

fn run_summary(root: PathBuf, failure_limit: usize) -> i32 {
    if !root.exists() {
        eprintln!("path does not exist: {}", root.display());
        return 2;
    }

    let files = match collect_js_files(&root) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("failed to collect test files: {error}");
            return 2;
        }
    };

    let mut summary = Summary::default();
    let mut failure_messages: BTreeMap<String, usize> = BTreeMap::new();

    for path in files {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("failed to read {}: {error}", path.display());
                return 2;
            }
        };

        let metadata = extract_metadata(&source);
        summary.total += 1;
        if metadata.is_module {
            summary.module_tests += 1;
        }
        if metadata.expectation == Expectation::ParseFail {
            summary.expected_parse_fail += 1;
        }

        if metadata.expectation == Expectation::ParseFail {
            match parse_with_options(
                &source,
                ParseOptions {
                    is_module: metadata.is_module,
                    is_strict: metadata.is_strict,
                },
            ) {
                Ok(_) => {
                    summary.unexpected_parse_successes.push(path);
                }
                Err(_) => {
                    summary.negative_cases_matched += 1;
                    summary.passed += 1;
                }
            }
            continue;
        }

        let program = match parse_with_options(
            &source,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(program) => program,
            Err(error) => {
                *failure_messages.entry(error.message.clone()).or_default() += 1;
                summary.failures.push(FailureRecord {
                    path,
                    stage: "parse",
                    message: error.to_string(),
                });
                continue;
            }
        };

        let emitted = program_to_js(&program);
        let reparsed = match parse_with_options(
            &emitted,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(program) => program,
            Err(error) => {
                *failure_messages.entry(error.message.clone()).or_default() += 1;
                summary.failures.push(FailureRecord {
                    path,
                    stage: "reparse_emitted",
                    message: format!("{error}\nemitted:\n{emitted}"),
                });
                continue;
            }
        };

        let re_emitted = program_to_js(&reparsed);
        if re_emitted != emitted {
            let reason = "re-emitted source did not stabilize after one round-trip".to_string();
            *failure_messages.entry(reason.clone()).or_default() += 1;
            summary.failures.push(FailureRecord {
                path,
                stage: "stability",
                message: format!("{}\nfirst:\n{}\nsecond:\n{}", reason, emitted, re_emitted),
            });
            continue;
        }

        summary.positive_cases_matched += 1;
        summary.passed += 1;
    }

    let failed = summary.total - summary.passed;
    let percent = if summary.total == 0 {
        100.0
    } else {
        (summary.passed as f64 * 100.0) / summary.total as f64
    };
    let parse_pass_total = summary.total - summary.expected_parse_fail;

    println!("Test262 AST -> JS round-trip check");
    println!("root: {}", root.display());
    println!("total: {}", summary.total);
    println!("passed: {}", summary.passed);
    println!("failed: {}", failed);
    println!("pass rate: {:.2}%", percent);
    println!("parse-negative tests: {}", summary.expected_parse_fail);
    println!("module-flag tests: {}", summary.module_tests);
    println!(
        "positive round-trip cases matched: {}/{}",
        summary.positive_cases_matched, parse_pass_total
    );
    println!(
        "negative parse cases matched: {}/{}",
        summary.negative_cases_matched, summary.expected_parse_fail
    );
    println!(
        "unexpected parse successes: {}",
        summary.unexpected_parse_successes.len()
    );
    println!("unexpected failures: {}", summary.failures.len());

    if !summary.failures.is_empty() {
        println!();
        println!("top unexpected failure messages:");
        let mut top_messages: Vec<_> = failure_messages.into_iter().collect();
        top_messages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (message, count) in top_messages.into_iter().take(failure_limit) {
            println!("  {count:>5}  {message}");
        }

        println!();
        println!("sample unexpected failures:");
        for failure in summary.failures.iter().take(failure_limit) {
            println!(
                "  {} [{}] :: {}",
                failure.path.display(),
                failure.stage,
                failure.message
            );
        }
    }

    if !summary.unexpected_parse_successes.is_empty() {
        println!();
        println!("sample unexpected parse successes:");
        for path in summary
            .unexpected_parse_successes
            .iter()
            .take(failure_limit)
        {
            println!("  {}", path.display());
        }
    }

    if failed > 0 {
        return 1;
    }

    0
}

fn default_test262_root() -> PathBuf {
    let workspace_test262 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test262/test");
    if Path::new("test262/test").exists() {
        PathBuf::from("test262/test")
    } else {
        workspace_test262
    }
}

fn run_jsonl(root: PathBuf) -> i32 {
    if !root.exists() {
        eprintln!("path does not exist: {}", root.display());
        return 2;
    }

    let files = match collect_js_files(&root) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("failed to collect test files: {error}");
            return 2;
        }
    };

    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    for path in files {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("failed to read {}: {error}", path.display());
                return 2;
            }
        };

        let metadata = extract_metadata(&source);
        if metadata.expectation == Expectation::ParseFail {
            let matched = parse_with_options(
                &source,
                ParseOptions {
                    is_module: metadata.is_module,
                    is_strict: metadata.is_strict,
                },
            )
            .is_err();

            if write_jsonl_record(
                &mut writer,
                &path,
                &metadata,
                "negative",
                matched,
                None,
                None,
            )
            .is_err()
            {
                eprintln!("failed to write jsonl output");
                return 2;
            }
            continue;
        }

        let program = match parse_with_options(
            &source,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(program) => program,
            Err(error) => {
                if write_jsonl_record(
                    &mut writer,
                    &path,
                    &metadata,
                    "parse",
                    false,
                    None,
                    Some(&error.to_string()),
                )
                .is_err()
                {
                    eprintln!("failed to write jsonl output");
                    return 2;
                }
                continue;
            }
        };

        let emitted = program_to_js(&program);
        if write_jsonl_record(
            &mut writer,
            &path,
            &metadata,
            "emit",
            true,
            Some(&emitted),
            None,
        )
        .is_err()
        {
            eprintln!("failed to write jsonl output");
            return 2;
        }
    }

    if writer.flush().is_err() {
        eprintln!("failed to flush jsonl output");
        return 2;
    }

    0
}

fn write_jsonl_record(
    writer: &mut impl Write,
    path: &Path,
    metadata: &Metadata,
    stage: &str,
    matched: bool,
    emitted: Option<&str>,
    error: Option<&str>,
) -> io::Result<()> {
    write!(
        writer,
        "{{\"path\":{},\"expectation\":\"{}\",\"is_module\":{},\"is_strict\":{},\"stage\":\"{}\",\"matched\":{}",
        json_string(&portable_path(path)),
        match metadata.expectation {
            Expectation::ParsePass => "parse-pass",
            Expectation::ParseFail => "parse-fail",
        },
        metadata.is_module,
        metadata.is_strict,
        stage,
        matched
    )?;

    if let Some(emitted) = emitted {
        write!(writer, ",\"emitted\":{}", json_string(emitted))?;
    }
    if let Some(error) = error {
        write!(writer, ",\"error\":{}", json_string(error))?;
    }

    writer.write_all(b"}\n")
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch.is_control() => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn collect_js_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                stack.push(entry?.path());
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("js")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("_FIXTURE"))
        {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn extract_metadata(source: &str) -> Metadata {
    let frontmatter = source
        .find("/*---")
        .and_then(|start| source[start + 5..].split_once("---*/"))
        .map(|(frontmatter, _)| frontmatter)
        .unwrap_or("");

    let mut is_module = false;
    let mut is_strict = false;
    let mut in_negative = false;
    let mut in_flags = false;
    let mut negative_phase = None::<String>;

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        if in_flags {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_flags = false;
            } else if let Some(flag) = trimmed.strip_prefix('-') {
                match flag.trim() {
                    "module" => is_module = true,
                    "onlyStrict" => is_strict = true,
                    _ => {}
                }
                continue;
            }
        }

        if trimmed == "negative:" {
            in_negative = true;
            continue;
        }

        if in_negative {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_negative = false;
            } else if let Some(value) = trimmed.strip_prefix("phase:") {
                negative_phase = Some(value.trim().to_string());
                continue;
            }
        }

        if trimmed == "flags:" {
            in_flags = true;
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("flags: [") {
            in_flags = false;
            for flag in value.trim_end_matches(']').split(',') {
                match flag.trim() {
                    "module" => is_module = true,
                    "onlyStrict" => is_strict = true,
                    _ => {}
                }
            }
            continue;
        }
    }

    let expectation = match negative_phase.as_deref() {
        Some("parse") | Some("early") => Expectation::ParseFail,
        _ => Expectation::ParsePass,
    };

    Metadata {
        expectation,
        is_module,
        is_strict,
    }
}
