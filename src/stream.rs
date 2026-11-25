//! `agent-phone stream`: a live view of the simulator in the browser, served on localhost.
//! Frames come straight from the framebuffer as MJPEG; clicking the picture taps the phone and
//! dragging swipes. Every request needs the random token printed at start, so other web pages
//! open in the same browser can't drive the phone.

use crate::native;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>agent-phone</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #111; font: 13px -apple-system, system-ui, sans-serif; color: #bbb; }
  main { display: grid; gap: 10px; justify-items: center; padding: 16px; }
  img { height: min(86vh, 900px); max-width: 100%; border-radius: 28px; box-shadow: 0 8px 40px #000a; cursor: crosshair; user-select: none; -webkit-user-drag: none; }
  #status { min-height: 1.4em; }
</style></head>
<body><main>
  <img id="screen" alt="Simulator screen" draggable="false">
  <div id="status">Click to tap, drag to swipe.</div>
</main>
<script>
  const token = new URLSearchParams(location.search).get("token");
  const img = document.getElementById("screen");
  const status = document.getElementById("status");
  img.src = "/mjpeg?token=" + encodeURIComponent(token);
  let down = null;
  const toPoints = (e) => {
    const r = img.getBoundingClientRect();
    return [ (e.clientX - r.left) / r.width * POINTS_W, (e.clientY - r.top) / r.height * POINTS_H ];
  };
  img.addEventListener("pointerdown", (e) => { down = { p: toPoints(e), t: Date.now() }; });
  img.addEventListener("pointerup", async (e) => {
    if (!down) return;
    const [x0, y0] = down.p, [x1, y1] = toPoints(e), held = Date.now() - down.t;
    down = null;
    const moved = Math.hypot(x1 - x0, y1 - y0) > 8;
    const q = moved ? `x=${x0}&y=${y0}&x2=${x1}&y2=${y1}` : `x=${x0}&y=${y0}&hold=${held > 450 ? held : 0}`;
    const r = await fetch(`/touch?token=${encodeURIComponent(token)}&${q}`, { method: "POST" });
    status.textContent = r.ok ? (moved ? "swiped" : held > 450 ? "long-pressed" : "tapped") + ` at ${Math.round(x0)},${Math.round(y0)}` : "failed: " + await r.text();
  });
</script>
</body></html>"#;

pub struct Options {
    pub port: u16,
    pub fps: f64,
    pub quality: f64,
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn query(path: &str) -> Vec<(String, String)> {
    path.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%20", " ")))
        .collect()
}

fn token() -> String {
    let mut bytes = [0u8; 12];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn handle(mut stream: TcpStream, target: native::Target, secret: &str, options: &Options, page: &str) -> Result<()> {
    let mut line = String::new();
    let mut reader = BufReader::new(stream.try_clone()?);
    reader.read_line(&mut line)?;
    // Skip the headers.
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? <= 2 {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let params = query(path);
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let route = path.split('?').next().unwrap_or("/");
    if get("token") != Some(secret) {
        return Ok(respond(&mut stream, "403 Forbidden", "text/plain", b"open the link agent-phone printed (it has the token)")?);
    }
    match (method, route) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html; charset=utf-8", page.as_bytes())?,
        ("GET", "/frame.jpg") => {
            let frame = target.frame(options.quality)?;
            respond(&mut stream, "200 OK", "image/jpeg", &frame)?;
        }
        ("GET", "/mjpeg") => {
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n--frame\r\n")?;
            let gap = Duration::from_secs_f64(1.0 / options.fps.clamp(1.0, 60.0));
            let mut last: Vec<u8> = Vec::new();
            let mut sent = Instant::now() - Duration::from_secs(10);
            loop {
                let started = Instant::now();
                let frame = target.frame(options.quality)?;
                // Unchanged screens still get a frame each second so the browser keeps the stream.
                if frame != last || sent.elapsed() > Duration::from_secs(1) {
                    // The boundary goes right after the frame: browsers show a part only once
                    // the next boundary arrives.
                    write!(stream, "Content-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", frame.len())?;
                    stream.write_all(&frame)?;
                    stream.write_all(b"\r\n--frame\r\n")?;
                    stream.flush()?;
                    last = frame;
                    sent = Instant::now();
                }
                if let Some(rest) = gap.checked_sub(started.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        }
        ("POST", "/touch") => {
            let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
            let (Some(x), Some(y)) = (num("x"), num("y")) else {
                return Ok(respond(&mut stream, "400 Bad Request", "text/plain", b"x and y are required")?);
            };
            let result = match (num("x2"), num("y2")) {
                (Some(x2), Some(y2)) => {
                    let points: Vec<(f64, f64)> = (0..=12).map(|i| {
                        let t = i as f64 / 12.0;
                        (x + (x2 - x) * t, y + (y2 - y) * t)
                    }).collect();
                    target.touch(&points, 0, 16)
                }
                _ => target.touch(&[(x, y)], (num("hold").unwrap_or(0.0) as u32).max(30), 0),
            };
            match result {
                Ok(()) => respond(&mut stream, "200 OK", "text/plain", b"ok")?,
                Err(e) => respond(&mut stream, "500 Internal Server Error", "text/plain", format!("{e:#}").as_bytes())?,
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found")?,
    }
    Ok(())
}

/// Serves until interrupted. Prints the viewer's address (with its token) first.
pub fn serve(dev_dir: &str, udid: &str, options: Options) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", options.port)).with_context(|| format!("port {} is in use; pass --port", options.port))?;
    let screen = native::Target::new(dev_dir, udid)?.screen()?;
    let page = PAGE.replace("POINTS_W", &screen.width.to_string()).replace("POINTS_H", &screen.height.to_string());
    let secret = token();
    let port = listener.local_addr()?.port();
    println!("streaming at http://127.0.0.1:{port}/?token={secret}  (Ctrl-C to stop)");
    std::io::stdout().flush()?;
    let options = std::sync::Arc::new(options);
    let (secret, page) = (std::sync::Arc::new(secret), std::sync::Arc::new(page));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let target = native::Target::new(dev_dir, udid)?;
        let (options, secret, page) = (options.clone(), secret.clone(), page.clone());
        std::thread::spawn(move || {
            let _ = handle(stream, target, &secret, &options, &page);
        });
    }
    Ok(())
}
