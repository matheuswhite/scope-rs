use crate::plugin::{Plugin, PluginRequestResult};
use crate::text::TextView;
use std::io::{BufRead, BufReader, Lines, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tui::backend::Backend;

pub struct ProcessRunner<B>
where
    B: Backend + Sync + Send + 'static,
{
    text_view: Arc<Mutex<TextView<B>>>,
}

impl<B> Clone for ProcessRunner<B>
where
    B: Backend + Sync + Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            text_view: self.text_view.clone(),
        }
    }
}

impl<B> ProcessRunner<B>
where
    B: Backend + Sync + Send + 'static,
{
    pub fn new(text_view: Arc<Mutex<TextView<B>>>) -> Self {
        Self { text_view }
    }

    pub fn run(&self, plugin_name: String, cmd: String) -> Result<PluginRequestResult, String> {
        Plugin::println(self.text_view.clone(), plugin_name.clone(), cmd.clone());

        let mut child = if cfg!(target_os = "windows") {
            unimplemented!()
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
        let stdout_pipe = Self::spawn_read_pipe(&IS_END, stdout, move |line| {
            Plugin::println(text_view2.clone(), plugin_name.clone(), line.clone());
        });

        let stderr = child.stderr.take().ok_or("Cannot get stderr".to_string())?;
        let stderr = BufReader::new(stderr).lines();
        let stderr_pipe = Self::spawn_read_pipe(&IS_END, stderr, move |line| {
            Plugin::eprintln(text_view3.clone(), plugin_name2.clone(), line)
        });

        let _ = child.wait();
        IS_END.store(true, Ordering::SeqCst);

        Ok(PluginRequestResult::Exec {
            stdout: stdout_pipe.join().unwrap(),
            stderr: stderr_pipe.join().unwrap(),
        })
    }

    fn spawn_read_pipe<P>(
        is_end: &'static AtomicBool,
        mut pipe: Lines<BufReader<P>>,
        mut print_fn: impl FnMut(String) + Send + 'static,
    ) -> JoinHandle<Vec<String>>
    where
        P: Read + Send + 'static,
    {
        std::thread::spawn(move || {
            let mut buffer = vec![];

            while is_end.load(Ordering::SeqCst) {
                if let Some(Ok(line)) = pipe.next() {
                    buffer.push(line.clone());
                    print_fn(line);
                }
            }

            while let Some(Ok(line)) = pipe.next() {
                buffer.push(line.clone());
                print_fn(line);
            }

            buffer
        })
    }
}
