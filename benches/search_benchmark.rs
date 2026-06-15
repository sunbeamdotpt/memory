use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use sunbeam_memory::config::MemoryConfig;
use sunbeam_memory::memory::service::MemoryService;

// ── query patterns ───────────────────────────────────────────────────────────

const ALICE_KEYWORD: &[&str] = &["rabbit", "queen", "tea party", "caterpillar"];
const ALICE_SEMANTIC: &[&str] = &[
    "Where did Alice fall down a hole?",
    "disappearing cat",
    "the pool of tears",
];
const ALICE_HYBRID: &[&str] = &[
    "Alice meets the Queen",
    "mad hatter tea party",
    "Rabbit sends a little Bill",
];

const CODE_KEYWORD: &[&str] = &[
    "search_facts",
    "SemanticDB",
    "fused_search",
    "MemoryService",
];
const CODE_SEMANTIC: &[&str] = &[
    "how does fused search work",
    "memory service adding facts",
    "embedding model dimensions",
];

const CROSS_CORPUS: &[&str] = &[
    "Alice search",
    "fact adventure",
    "memory rabbit",
    "store fact",
    "query embedding",
];

fn all_queries() -> Vec<&'static str> {
    ALICE_KEYWORD
        .iter()
        .chain(ALICE_SEMANTIC.iter())
        .chain(ALICE_HYBRID.iter())
        .chain(CODE_KEYWORD.iter())
        .chain(CODE_SEMANTIC.iter())
        .chain(CROSS_CORPUS.iter())
        .copied()
        .collect()
}

// ── corpus loading ───────────────────────────────────────────────────────────

/// Parse alice-in-wonderland.txt into chapter documents.
fn load_alice_chapters() -> Vec<(String, String)> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let text = std::fs::read_to_string(Path::new(&manifest_dir).join("alice-in-wonderland.txt"))
        .expect("alice-in-wonderland.txt not found in project root");

    let lines: Vec<&str> = text.lines().collect();
    let mut chapters = Vec::new();
    let mut current_start: Option<usize> = None;
    let mut current_title = String::new();

    for (i, line) in lines.iter().enumerate() {
        // Only match actual chapter headers (start of line, not indented table of contents)
        if line.starts_with("CHAPTER ") && line.contains(".") {
            // Save previous chapter
            if let Some(start) = current_start {
                let body = lines[start..i].join("\n");
                chapters.push((current_title.clone(), body));
            }
            current_start = Some(i);
            current_title = line.trim().to_string();
        }
    }

    // Save last chapter (up to THE END)
    if let Some(start) = current_start {
        let end = lines
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, l)| l.trim() == "THE END")
            .map(|(i, _)| i)
            .unwrap_or(lines.len());
        let body = lines[start..end].join("\n");
        chapters.push((current_title, body));
    }

    chapters
}

/// Find all .rs files under src/ and read their contents.
fn load_code_files() -> Vec<(String, String)> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let root = Path::new(&manifest_dir);
    let pattern = root.join("src").join("**").join("*.rs");
    let mut files = Vec::new();

    for entry in glob::glob(pattern.to_str().unwrap()).expect("invalid glob") {
        let path = entry.expect("glob error");
        let content = std::fs::read_to_string(&path).expect("read error");
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        files.push((name, content));
    }

    files
}

// ── indexing ─────────────────────────────────────────────────────────────────

async fn index_corpus(memory: &MemoryService) -> (usize, usize) {
    let alice = load_alice_chapters();
    let code = load_code_files();

    let alice_docs = alice.len();
    let code_docs = code.len();

    eprintln!(
        "\n[benchmark] Indexing {} Alice chapters + {} code files...",
        alice_docs, code_docs
    );

    for (title, body) in alice {
        memory
            .add_fact("alice", &body, Some(&title))
            .await
            .expect("add alice chapter");
    }

    for (name, content) in code {
        memory
            .add_fact("code", &content, Some(&name))
            .await
            .expect("add code file");
    }

    eprintln!("[benchmark] Indexing complete.\n");
    (alice_docs, code_docs)
}

// ── benchmarks ───────────────────────────────────────────────────────────────

fn bench_latency(c: &mut Criterion, rt: &tokio::runtime::Runtime, memory: &MemoryService) {
    let queries = all_queries();
    let mut group = c.benchmark_group("search/latency");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    for q in queries {
        group.bench_with_input(BenchmarkId::from_parameter(q), q, |b, query| {
            b.to_async(rt)
                .iter(|| async { memory.search_facts(query, 10, None).await.unwrap() });
        });
    }

    group.finish();
}

fn bench_filtered(c: &mut Criterion, rt: &tokio::runtime::Runtime, memory: &MemoryService) {
    let queries = ["rabbit", "search_facts", "Alice search", "store fact"];
    let mut group = c.benchmark_group("search/filtered");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    for q in queries {
        group.bench_with_input(BenchmarkId::new("alice", q), q, |b, query| {
            b.to_async(rt)
                .iter(|| async { memory.search_facts(query, 10, Some("alice")).await.unwrap() });
        });
        group.bench_with_input(BenchmarkId::new("code", q), q, |b, query| {
            b.to_async(rt)
                .iter(|| async { memory.search_facts(query, 10, Some("code")).await.unwrap() });
        });
    }

    group.finish();
}

// ── consistency check ────────────────────────────────────────────────────────

async fn check_consistency(memory: &MemoryService) {
    let queries = all_queries();
    let runs = 5;

    eprintln!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    eprintln!(
        "║                        CONSISTENCY REPORT ({} runs)                          ║",
        runs
    );
    eprintln!("╠══════════════════════════════════════════════════════════════════════════════╣");
    eprintln!(
        "{:<30} {:>6} {:>12} {:>20}",
        "Query", "Pass", "TopNSame", "NsDistribution"
    );
    eprintln!("──────────────────────────────────────────────────────────────────────────────");

    let mut total_pass = 0;
    let mut total_fail = 0;

    for q in queries {
        let mut all_top3: Vec<Vec<String>> = Vec::with_capacity(runs);
        let mut all_top5_ns: Vec<Vec<String>> = Vec::with_capacity(runs);

        for _ in 0..runs {
            let results = memory.search_facts(q, 10, None).await.unwrap();
            let top3: Vec<String> = results.iter().take(3).map(|r| r.id.clone()).collect();
            let top5_ns: Vec<String> = results
                .iter()
                .take(5)
                .map(|r| r.namespace.clone())
                .collect();
            all_top3.push(top3);
            all_top5_ns.push(top5_ns);
        }

        // Check if all top-3 ID lists are identical
        let first = &all_top3[0];
        let same = all_top3.iter().all(|v| v == first);

        // Namespace distribution (use first run's top-5)
        let mut ns_counts = HashMap::new();
        for ns in &all_top5_ns[0] {
            *ns_counts.entry(ns.as_str()).or_insert(0) += 1;
        }
        let ns_dist: Vec<String> = ns_counts
            .iter()
            .map(|(k, v)| format!("{}:{}", k, v))
            .collect();

        let pass_str = if same { "PASS" } else { "FAIL" };
        if same {
            total_pass += 1;
        } else {
            total_fail += 1;
        }

        let truncated = if q.len() > 28 { &q[..28] } else { q };
        eprintln!(
            "{:<30} {:>6} {:>12} {:>20}",
            truncated,
            pass_str,
            if same { "3/3" } else { "varies" },
            ns_dist.join(", ")
        );
    }

    eprintln!("──────────────────────────────────────────────────────────────────────────────");
    eprintln!(
        "Result: {}/{} passed ({:.0}%)\n",
        total_pass,
        total_pass + total_fail,
        100.0 * total_pass as f64 / (total_pass + total_fail) as f64
    );
}

// ── main ─────────────────────────────────────────────────────────────────────

fn criterion_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };

    eprintln!("\n[benchmark] Creating MemoryService (model may download on first run)...");
    let memory = rt
        .block_on(MemoryService::new(&config))
        .expect("MemoryService");

    let (alice_docs, code_docs) = rt.block_on(index_corpus(&memory));
    eprintln!(
        "[benchmark] Corpus: {} Alice chapters, {} code files\n",
        alice_docs, code_docs
    );

    bench_latency(c, &rt, &memory);
    bench_filtered(c, &rt, &memory);

    // Consistency check is not timed by Criterion; we just print a report
    rt.block_on(check_consistency(&memory));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
