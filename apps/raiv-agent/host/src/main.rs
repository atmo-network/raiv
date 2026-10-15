//! Native headless composition root; no host agent decisions or effect execution.

#[cfg(target_os = "linux")]
mod app;

fn main() {
    #[cfg(target_os = "linux")]
    match app::run(std::env::args_os().skip(1).take(14).collect()) {
        Ok(output) => {
            use std::io::Write;
            // A closed reader (for example `status | head`) is not a command failure.
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = stdout
                .write_all(output.as_bytes())
                .and_then(|()| stdout.flush())
            {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("raiv-agent: output failed: {error}");
                    std::process::exit(2);
                }
            }
        }
        Err(error) => {
            eprintln!("raiv-agent: {error}");
            std::process::exit(2);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("raiv-agent headless disk application currently requires Linux");
        std::process::exit(2);
    }
}
