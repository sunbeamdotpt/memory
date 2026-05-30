use std::fs::OpenOptions;
use std::io::Write;

fn log(msg: &str) {
    let mut f = OpenOptions::new().create(true).append(true).open("/tmp/sunbeam_debug.log").unwrap();
    writeln!(f, "[{}] {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(), msg).unwrap();
}

#[tokio::main]
async fn main() {
    log("starting");
    let config = mcp_server::config::MemoryConfig::from_env();
    log(&format!("base_dir={}", config.base_dir));
    match mcp_server::memory::service::MemoryService::new(&config).await {
        Ok(memory) => {
            log("memory service ready");
            let (event_tx, event_rx) = crossbeam_channel::bounded(1000);
            let watcher = mcp_server::indexer::IndexWatcher::new(event_tx).expect("failed to create file watcher");
            let indexer = mcp_server::indexer::IndexService::new(memory.clone(), event_rx, watcher);
            let indexer_for_mcp = indexer.clone();
            tokio::spawn(indexer.run());
            log("indexer spawned");
            
            let stdin = tokio::io::BufReader::new(tokio::io::stdin());
            let mut stdout = tokio::io::stdout();
            let mut lines = stdin.lines();
            log("entering loop");
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        log(&format!("got line: {}", line.chars().take(50).collect::<String>()));
                        if line.trim().is_empty() { continue; }
                        let req: Result<mcp_server::mcp::protocol::Request, _> = serde_json::from_str(&line);
                        match req {
                            Ok(request) => {
                                log(&format!("method={}", request.method));
                                let resp = mcp_server::mcp::server::handle(&request, &memory, &indexer_for_mcp).await;
                                if let Some(r) = resp {
                                    let json = serde_json::to_string(&r).unwrap();
                                    log(&format!("sending response id={}", r.id));
                                    let _ = stdout.write_all(json.as_bytes()).await;
                                    let _ = stdout.write_all(b"\n").await;
                                    let _ = stdout.flush().await;
                                }
                            }
                            Err(e) => {
                                log(&format!("parse error: {}", e));
                                let err = mcp_server::mcp::protocol::Response::err(serde_json::Value::Null, -32700, e.to_string());
                                let json = serde_json::to_string(&err).unwrap();
                                let _ = stdout.write_all(json.as_bytes()).await;
                                let _ = stdout.write_all(b"\n").await;
                                let _ = stdout.flush().await;
                            }
                        }
                    }
                    Ok(None) => { log("EOF"); break; }
                    Err(e) => { log(&format!("stdin error: {}", e)); break; }
                }
            }
            log("exiting loop");
        }
        Err(e) => {
            log(&format!("memory service failed: {}", e));
        }
    }
    log("done");
}
