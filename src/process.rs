use crate::plugin::{Plugin, PluginRequestResult};
use crate::text::TextView;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task;

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
        stop_process_flag: Arc<AtomicBool>,
    ) -> Result<PluginRequestResult, String> {
        Plugin::println(self.text_view.clone(), plugin_name.clone(), cmd.clone()).await;

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

        static IS_END: AtomicBool = AtomicBool::new(false);
        IS_END.store(false, Ordering::SeqCst);
        let text_view2 = self.text_view.clone();
        let text_view3 = self.text_view.clone();
        let plugin_name2 = plugin_name.clone();
        let stop_process_flag2 = stop_process_flag.clone();
        let stop_process_flag3 = stop_process_flag.clone();

        let stdout = child.stdout.take().ok_or("Cannot get stdout".to_string())?;
        let mut stdout = BufReader::new(stdout).lines();
        let stdout_pipe = task::spawn_local(async move {
            let mut buffer = vec![];

            while IS_END.load(Ordering::SeqCst) {
                if stop_process_flag.load(Ordering::SeqCst) {
                    return buffer;
                }

                if let Some(Ok(line)) = stdout.next() {
                    buffer.push(line.clone());
                    Plugin::println(text_view2.clone(), plugin_name.clone(), line.clone()).await;
                }
            }

            while let Some(Ok(line)) = stdout.next() {
                if stop_process_flag.load(Ordering::SeqCst) {
                    return buffer;
                }

                buffer.push(line.clone());
                Plugin::println(text_view2.clone(), plugin_name.clone(), line.clone()).await;
            }

            buffer
        });

        let stderr = child.stderr.take().ok_or("Cannot get stderr".to_string())?;
        let mut stderr = BufReader::new(stderr).lines();
        let stderr_pipe = task::spawn_local(async move {
            let mut buffer = vec![];

            while IS_END.load(Ordering::SeqCst) {
                if stop_process_flag2.load(Ordering::SeqCst) {
                    return buffer;
                }

                if let Some(Ok(line)) = stderr.next() {
                    buffer.push(line.clone());
                    Plugin::eprintln(text_view3.clone(), plugin_name2.clone(), line).await;
                }
            }

            while let Some(Ok(line)) = stderr.next() {
                if stop_process_flag2.load(Ordering::SeqCst) {
                    return buffer;
                }

                buffer.push(line.clone());
                Plugin::eprintln(text_view3.clone(), plugin_name2.clone(), line).await;
            }

            buffer
        });

        'wait_loop: while let Ok(None) = child.try_wait() {
            if stop_process_flag3.load(Ordering::SeqCst) {
                let _ = child.kill();
                stdout_pipe.abort();
                stderr_pipe.abort();
                break 'wait_loop;
            }
        }

        IS_END.store(true, Ordering::SeqCst);

        Ok(PluginRequestResult::Exec {
            stdout: stdout_pipe.await.unwrap(),
            stderr: stderr_pipe.await.unwrap(),
        })
    }
}
