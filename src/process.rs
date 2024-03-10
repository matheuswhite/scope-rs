use crate::plugin::{Plugin, PluginRequestResult};
use crate::text::TextView;
use std::io::{BufRead, BufReader, Lines, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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

    pub fn run(
        &self,
        plugin_name: String,
        cmd: String,
        stop_process_flag: Arc<AtomicBool>,
    ) -> Result<PluginRequestResult, String> {
        Plugin::println(self.text_view.clone(), plugin_name.clone(), cmd.clone());

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

        let stdout = child.stdout.take().ok_or("Cannot get stdout".to_string())?;
        let stdout = BufReader::new(stdout).lines();
        let stdout_pipe =
            Self::spawn_read_pipe(&IS_END, stop_process_flag.clone(), stdout, move |line| {
                Plugin::println(text_view2.clone(), plugin_name.clone(), line.clone());
            });

        let stderr = child.stderr.take().ok_or("Cannot get stderr".to_string())?;
        let stderr = BufReader::new(stderr).lines();
        let stderr_pipe =
            Self::spawn_read_pipe(&IS_END, stop_process_flag.clone(), stderr, move |line| {
                Plugin::eprintln(text_view3.clone(), plugin_name2.clone(), line)
            });

        'wait_loop: while let Ok(None) = child.try_wait() {
            if stop_process_flag.load(Ordering::SeqCst) {
                let _ = child.kill();
                break 'wait_loop;
            }
        }

        IS_END.store(true, Ordering::SeqCst);

        Ok(PluginRequestResult::Exec {
            stdout: stdout_pipe.join().unwrap(),
            stderr: stderr_pipe.join().unwrap(),
        })
    }

    fn spawn_read_pipe<P>(
        is_end: &'static AtomicBool,
        stop_process_flag: Arc<AtomicBool>,
        mut pipe: Lines<BufReader<P>>,
        mut print_fn: impl FnMut(String) + Send + 'static,
    ) -> JoinHandle<Vec<String>>
    where
        P: Read + Send + 'static,
    {
        std::thread::spawn(move || {
            let mut buffer = vec![];

            while is_end.load(Ordering::SeqCst) {
                if stop_process_flag.load(Ordering::SeqCst) {
                    return buffer;
                }

                if let Some(Ok(line)) = pipe.next() {
                    buffer.push(line.clone());
                    print_fn(line);
                }
            }

            while let Some(Ok(line)) = pipe.next() {
                if stop_process_flag.load(Ordering::SeqCst) {
                    return buffer;
                }

                buffer.push(line.clone());
                print_fn(line);
            }

            buffer
        })
    }
}
