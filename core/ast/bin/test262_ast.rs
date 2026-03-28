use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process,
};

use ::ast::{ParseOptions, parse_with_options};

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
    message: String,
}

#[derive(Debug, Default)]
struct Summary {
    total: usize,
    passed: usize,
    expected_parse_fail: usize,
    module_tests: usize,
    unexpected_successes: Vec<PathBuf>,
    unexpected_failures: Vec<FailureRecord>,
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
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_test262_root);
    let failure_limit = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);

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

        match parse_with_options(
            &source,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(_) if metadata.expectation == Expectation::ParsePass => {
                summary.passed += 1;
            }
            Err(_error) if metadata.expectation == Expectation::ParseFail => {
                summary.passed += 1;
            }
            Ok(_) => {
                summary.unexpected_successes.push(path);
            }
            Err(error) => {
                *failure_messages.entry(error.message.clone()).or_default() += 1;
                summary.unexpected_failures.push(FailureRecord {
                    path,
                    message: error.to_string(),
                });
            }
        }
    }

    let failed = summary.total - summary.passed;
    let percent = if summary.total == 0 {
        100.0
    } else {
        (summary.passed as f64 * 100.0) / summary.total as f64
    };
    let parse_pass_total = summary.total - summary.expected_parse_fail;
    let parse_fail_total = summary.expected_parse_fail;
    let parse_pass_matched = parse_pass_total.saturating_sub(summary.unexpected_failures.len());
    let parse_fail_matched = parse_fail_total.saturating_sub(summary.unexpected_successes.len());

    println!("Test262 AST parse check");
    println!("root: {}", root.display());
    println!("total: {}", summary.total);
    println!("passed: {}", summary.passed);
    println!("failed: {}", failed);
    println!("pass rate: {:.2}%", percent);
    println!("parse-negative tests: {}", summary.expected_parse_fail);
    println!("module-flag tests: {}", summary.module_tests);
    println!(
        "positive parse cases matched: {}/{}",
        parse_pass_matched, parse_pass_total
    );
    println!(
        "negative parse cases matched: {}/{}",
        parse_fail_matched, parse_fail_total
    );
    println!(
        "unexpected parse successes: {}",
        summary.unexpected_successes.len()
    );
    println!(
        "unexpected parse failures: {}",
        summary.unexpected_failures.len()
    );

    if !summary.unexpected_failures.is_empty() {
        println!();
        println!("top unexpected failure messages:");
        let mut top_messages: Vec<_> = failure_messages.into_iter().collect();
        top_messages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (message, count) in top_messages.into_iter().take(failure_limit) {
            println!("  {count:>5}  {message}");
        }

        println!();
        println!("sample unexpected failures:");
        for failure in summary.unexpected_failures.iter().take(failure_limit) {
            println!("  {} :: {}", failure.path.display(), failure.message);
        }
    }

    if !summary.unexpected_successes.is_empty() {
        println!();
        println!("sample unexpected parse successes:");
        for path in summary.unexpected_successes.iter().take(failure_limit) {
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

        if let Some(flags) = trimmed.strip_prefix("flags:") {
            in_flags = true;
            let flags = flags.trim();
            if let Some(flags) = flags
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                for flag in flags
                    .split(',')
                    .map(str::trim)
                    .filter(|flag| !flag.is_empty())
                {
                    match flag {
                        "module" => is_module = true,
                        "onlyStrict" => is_strict = true,
                        _ => {}
                    }
                }
                in_flags = false;
            } else if !flags.is_empty() {
                match flags {
                    "module" => is_module = true,
                    "onlyStrict" => is_strict = true,
                    _ => {}
                }
                in_flags = false;
            }
        }
    }

    let expectation = match negative_phase.as_deref() {
        Some("parse") => Expectation::ParseFail,
        _ => Expectation::ParsePass,
    };

    Metadata {
        expectation,
        is_module,
        is_strict: is_module || is_strict,
    }
}
