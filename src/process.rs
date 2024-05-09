use crate::plugin::{Plugin, PluginRequestResult};
use crate::text::TextView;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

pub struct ProcessRunner {
    text_view: Arc<Mutex<TextView>>,
}

impl Clone for ProcessRunner {
    fn clone(&self) -> Self {
        Self {
            text_view: self.text_view.clone(),
        }
    }
}

impl ProcessRunner {
    pub fn new(text_view: Arc<Mutex<TextView>>) -> Self {
        Self { text_view }
    }

    pub async fn run(
        &self,
        plugin_name: String,
        cmd: String,
        quiet: bool,
        stop_process_flag: Arc<AtomicBool>,
    ) -> Result<PluginRequestResult, String> {
        if !quiet {
            Plugin::println(self.text_view.clone(), plugin_name.clone(), cmd.clone()).await;
        }

        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .arg("/C")
                .arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .spawn()
                .map_err(|err| err.to_string())?
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|err| err.to_string())?
        };

        let text_view2 = self.text_view.clone();
        let text_view3 = self.text_view.clone();
        let plugin_name2 = plugin_name.clone();
        let stop_process_flag2 = stop_process_flag.clone();

        let stdout = child.stdout.take().ok_or("Cannot get stdout".to_string())?;
        let mut stdout = BufReader::new(stdout).lines();
        let stdout_pipe = tokio::spawn(async move {
            let mut buffer = vec![];

            while let Ok(Some(line)) = stdout.next_line().await {
                if stop_process_flag.load(Ordering::SeqCst) {
                    return buffer;
                }

                buffer.push(line.clone());
                if !quiet {
                    Plugin::println(text_view2.clone(), plugin_name.clone(), line.clone()).await;
                }
            }

            buffer
        });

        let stderr = child.stderr.take().ok_or("Cannot get stderr".to_string())?;
        let mut stderr = BufReader::new(stderr).lines();
        let stderr_pipe = tokio::spawn(async move {
            let mut buffer = vec![];

            while let Ok(Some(line)) = stderr.next_line().await {
                if stop_process_flag2.load(Ordering::SeqCst) {
                    return buffer;
                }

                buffer.push(line.clone());
                if !quiet {
                    Plugin::eprintln(text_view3.clone(), plugin_name2.clone(), line).await;
                }
            }

            buffer
        });

        Ok(PluginRequestResult::Exec {
            stdout: stdout_pipe.await.unwrap(),
            stderr: stderr_pipe.await.unwrap(),
        })
    }
}
