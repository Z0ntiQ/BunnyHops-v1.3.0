// Разделяем TCP / DNS / TLS / HTTP для диагностики исходящих из Rust.
use std::time::Duration;

#[tokio::main]
async fn main() {
    // 1. TCP к IP httpbin.org:443
    let t1 = tokio::time::timeout(Duration::from_secs(8), tokio::net::TcpStream::connect(("16.58.42.220", 443))).await;
    println!("1. raw TCP 16.58.42.220:443 -> {:?}", t1.map(|r| r.map(|_| "ok").map_err(|e| e.to_string())));

    // 2. DNS через tokio getaddrinfo
    let t2 = tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host("httpbin.org:443")).await;
    println!("2. lookup httpbin.org -> {:?}", t2.map(|r| r.map(|v| v.collect::<Vec<_>>()).map_err(|e| e.to_string())));

    // 3. plain HTTP (без TLS) к localhost-серверу? нет: к httpbin через reqwest
    let t3 = tokio::time::timeout(Duration::from_secs(8), reqwest::Client::new().get("http://example.com/").send()).await;
    println!("3. reqwest http://example.com -> {:?}", t3.map(|r| r.map(|r| r.status().as_u16()).map_err(|e| format!("{e:?}"))));

    // 4. TLS руками через rustls? нет: reqwest https к example.com
    let t4 = tokio::time::timeout(Duration::from_secs(8), reqwest::Client::new().get("https://example.com/").send()).await;
    println!("4. reqwest https://example.com -> {:?}", t4.map(|r| r.map(|r| r.status().as_u16()).map_err(|e| format!("{e:?}"))));
}
