use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn print_system_info() {
    println!("\n=== System Information ===");

    if let Ok(output) = std::process::Command::new("uname").arg("-a").output() {
        if let Ok(info) = String::from_utf8(output.stdout) {
            println!("OS: {}", info.trim());
        }
    }

    if let Ok(output) = std::process::Command::new("nproc").output() {
        if let Ok(cores) = String::from_utf8(output.stdout) {
            print!("CPU Cores: {}", cores.trim());
        }
    }

    if let Ok(output) = std::process::Command::new("sh")
        .arg("-c")
        .arg("grep 'model name' /proc/cpuinfo | head -1")
        .output()
    {
        if let Ok(model) = String::from_utf8(output.stdout) {
            if !model.is_empty() {
                println!(" ({})", model.trim().replace("model name\t: ", ""));
            }
        }
    }

    if let Ok(output) = std::process::Command::new("free").arg("-h").output() {
        if let Ok(mem) = String::from_utf8(output.stdout) {
            println!("Memory:\n{}", mem);
        }
    }

    println!("Rust: {}", env!("CARGO_PKG_VERSION"));
}

// Helper to spawn progress logging task
fn spawn_progress_logger(
    success_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
    start: Instant,
    total_requests: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let successes = success_count.load(Ordering::Relaxed);
            let errors = error_count.load(Ordering::Relaxed);
            let elapsed = start.elapsed().as_secs_f64();
            let current_rps = (successes + errors) as f64 / elapsed;
            let remaining = total_requests - (successes + errors) as usize;
            let eta = if current_rps > 0.0 {
                remaining as f64 / current_rps
            } else {
                0.0
            };
            println!(
                "  Progress: {} / {} | Errors: {} | RPS: {:.0} | ETA: {:.1}s",
                successes, total_requests, errors, current_rps, eta
            );
        }
    })
}

/// Load test for POST /api/links
/// Requires running server on http://localhost:8080
///
/// Run with: RUST_TEST_THREADS=1 cargo test --test load_test -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn load_test_create_links() {
    print_system_info();

    let client = reqwest::Client::new();
    let base_url = "http://localhost:8080";

    let concurrent_requests = 10_000;
    let requests_per_task = 25;
    let total_requests = concurrent_requests * requests_per_task;

    println!("\n=== Starting Create Links Load Test ===");
    println!("Concurrent tasks: {}", concurrent_requests);
    println!("Requests per task: {}", requests_per_task);
    println!("Total requests: {}", total_requests);

    let success_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    let progress_handle = spawn_progress_logger(
        Arc::clone(&success_count),
        Arc::clone(&error_count),
        start,
        total_requests,
    );

    let mut handles = vec![];

    for task_id in 0..concurrent_requests {
        let client = client.clone();
        let success = Arc::clone(&success_count);
        let errors = Arc::clone(&error_count);

        let handle = tokio::spawn(async move {
            for i in 0..requests_per_task {
                let url = format!("https://example.com/test?task={}&req={}", task_id, i);

                match client
                    .post(format!("{}/api/links", base_url))
                    .json(&serde_json::json!({ "url": url }))
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        let _ = handle.await;
    }

    progress_handle.abort();
    let elapsed = start.elapsed();
    let successes = success_count.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);
    let rps = total_requests as f64 / elapsed.as_secs_f64();

    println!("\n=== Load Test Results ===");
    println!("Total requests: {}", total_requests);
    println!("Successful: {}", successes);
    println!("Errors: {}", errors);
    println!("Duration: {:.2}s", elapsed.as_secs_f64());
    println!("Requests/sec: {:.2}", rps);
    println!(
        "Success rate: {:.1}%",
        (successes as f64 / total_requests as f64) * 100.0
    );

    assert_eq!(errors, 0, "All requests should succeed");
}

/// Stress test: high concurrency with random-access redirects
/// Creates multiple short URLs, then randomly accesses them to simulate realistic cache/index patterns
#[tokio::test]
#[ignore]
async fn load_test_redirects() {
    print_system_info();

    // Disable redirect following so we can see the 302 responses
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build client");
    let base_url = "http://localhost:8080";

    // First, create multiple short URLs to simulate real-world random access
    let num_codes_to_create = 10_000;
    let mut codes = Vec::new();

    println!("Creating {} short codes...", num_codes_to_create);
    for i in 0..num_codes_to_create {
        let create_response = client
            .post(format!("{}/api/links", base_url))
            .json(&serde_json::json!({
                "url": format!("https://example.com/target?id={}", i)
            }))
            .send()
            .await
            .expect("Failed to create link");

        let body: serde_json::Value = create_response
            .json()
            .await
            .expect("Failed to parse response");

        let code = body["code"]
            .as_str()
            .expect("Missing code in response")
            .to_string();
        codes.push(code);

        if (i + 1) % 100 == 0 {
            println!("  Created {} codes...", i + 1);
        }
    }

    println!("Created {} short codes, starting load test...", codes.len());

    // Now hammer with random-access redirect requests
    let concurrent_requests = 10_000;
    let requests_per_task = 25;
    let total_requests = concurrent_requests * requests_per_task;

    println!("\n=== Starting Redirect Load Test ===");
    println!("Concurrent tasks: {}", concurrent_requests);
    println!("Requests per task: {}", requests_per_task);
    println!("Total requests: {}", total_requests);

    let success_count = Arc::new(AtomicU64::new(0));
    let error_count = Arc::new(AtomicU64::new(0));
    let codes = Arc::new(codes);
    let start = Instant::now();

    let progress_handle = spawn_progress_logger(
        Arc::clone(&success_count),
        Arc::clone(&error_count),
        start,
        total_requests,
    );

    let mut handles = vec![];

    for task_id in 0..concurrent_requests {
        let client = client.clone();
        let codes = Arc::clone(&codes);
        let success = Arc::clone(&success_count);
        let errors = Arc::clone(&error_count);

        let handle = tokio::spawn(async move {
            // Seed random generator per task for variety
            let mut seed = task_id as u64;
            for _ in 0..requests_per_task {
                // Simple LCG for random index
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let idx = (seed as usize) % codes.len();
                let code = &codes[idx];

                match client.get(format!("{}/{}", base_url, code)).send().await {
                    Ok(response) if response.status() == 302 => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    progress_handle.abort();
    let elapsed = start.elapsed();
    let successes = success_count.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);
    let rps = total_requests as f64 / elapsed.as_secs_f64();

    println!("\n=== Redirect Load Test Results (Random-Access) ===");
    println!("Created codes: {}", codes.len());
    println!("Total requests: {}", total_requests);
    println!("Successful: {}", successes);
    println!("Errors: {}", errors);
    println!("Duration: {:.2}s", elapsed.as_secs_f64());
    println!("Requests/sec: {:.2}", rps);

    assert_eq!(errors, 0, "All redirects should succeed");
}
