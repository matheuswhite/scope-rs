use std::process::Stdio;
use tokio::process::Command;

pub struct Shell;

impl Shell {
    pub async fn run(cmd: String) -> Result<(String, String), String> {
        let child = if cfg!(target_os = "windows") {
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

        let output = child
            .wait_with_output()
            .await
            .map_err(|err| err.to_string())?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok((stdout.to_string(), stderr.to_string()))
    }

    pub async fn exist(program: String) -> bool {
        let mut child = if cfg!(target_os = "windows") {
            let Ok(res) = Command::new("cmd")
                .arg("/C")
                .arg(program)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .spawn()
            else {
                return false;
            };

            res
        } else {
            let Ok(res) = Command::new("sh")
                .arg("-c")
                .arg(program)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            else {
                return false;
            };

            res
        };

        let Ok(res) = child.wait().await else {
            return false;
        };

        res.success()
    }
}
