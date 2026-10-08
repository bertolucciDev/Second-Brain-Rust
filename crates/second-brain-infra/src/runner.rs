//! Adapter `CommandRunner` (P7): exec de processos com timeout via std.
//!
//! O legado usava `child_process` no MCP; aqui é std-only, cross-platform.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::{CommandOutput, CommandRunner, CommandSpec};

/// Executa um comando externo com timeout (padrão legado: `exec` no server.ts).
/// Sem shell intermediário — injeção de argumentos preservada pela std.
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, spec: &CommandSpec) -> Result<CommandOutput> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|e| (e.key.as_str(), e.value.as_str())))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &spec.cwd {
            command.current_dir(dir);
        }
        let mut child = command
            .spawn()
            .map_err(|e| AppError::Command(format!("falha ao iniciar {}: {e}", spec.program)))?;

        let deadline = spec.timeout_ms.map(Duration::from_millis);
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = String::new();
                    let mut stderr = String::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_string(&mut stdout);
                    }
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    return Ok(CommandOutput {
                        exit_code: status.code().unwrap_or(-1),
                        stdout,
                        stderr,
                    });
                }
                Ok(None) => {
                    if let Some(d) = deadline {
                        if start.elapsed() > d {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(AppError::Command(format!(
                                "timeout após {:?}: {}",
                                d, spec.program
                            )));
                        }
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(AppError::Command(format!("falha em wait: {e}")));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
            cwd: None,
            timeout_ms: Some(5000),
        }
    }

    #[test]
    fn runs_and_captures_exit_code() {
        let mut runner = ProcessRunner;
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C", "echo", "ok"])
        } else {
            ("sh", vec!["-c", "echo ok"])
        };
        let out = runner
            .run(&CommandSpec {
                program: program.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
                env: Vec::new(),
                cwd: None,
                timeout_ms: Some(5000),
            })
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.trim().ends_with("ok"));
    }

    #[test]
    fn failing_program_returns_error() {
        let mut runner = ProcessRunner;
        let out = runner.run(&spec("__definitely_not_a_program__", &[]));
        assert!(out.is_err());
    }
}
