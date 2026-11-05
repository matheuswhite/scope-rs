//! Shell-completion tests for `scope completions <SHELL>` (issues #231, #236).
//!
//! The script `scope completions` prints is a *registration*: it makes the
//! shell run `SCOPE_COMPLETE=<shell> scope -- <words>` on every Tab, and the
//! binary answers with the candidates. Three layers:
//!   1. Script tests — run `scope completions <SHELL>` and assert on what it
//!      prints. Portable: they need no shell at all, so they also guard Windows
//!      CI. These pin the two ways the subcommand can silently ship broken: the
//!      `See you later ^^` epilogue leaking into a script the shell sources, and
//!      the command being named after the *package* (`scope-monitor`) instead of
//!      the binary, which makes the completion never fire.
//!   2. Engine tests — make the call the script makes, and assert on the
//!      candidates: the CLI surface, and the values #236 is about (live ports,
//!      baud rates, probe-rs targets, file names for `--tag-file`). Also
//!      shell-free.
//!   3. Shell tests — actually ask bash / fish / zsh / PowerShell to complete
//!      `scope se` and a value, which is the acceptance criterion of the
//!      issues. Each is skipped when its shell is missing (no CI runner has all
//!      four), so they never fail for want of a shell.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Every shell `scope completions` advertises.
const SHELLS: [&str; 5] = ["bash", "elvish", "fish", "powershell", "zsh"];

/// The script `scope completions <shell>` prints, or a panic with its stderr.
fn script(shell: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["completions", shell])
        .output()
        .expect("spawn scope");
    assert!(
        out.status.success(),
        "`scope completions {shell}` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "`scope completions {shell}` wrote to stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("script is UTF-8")
}

// ------------------------------------------------------------------ scripts ---

/// Every advertised shell yields a non-empty script that registers the
/// completion for `scope` — not for `scope-monitor`. `clap_derive` names the
/// command after `CARGO_PKG_NAME`, so without `#[command(name = "scope")]` the
/// generated script targets a binary that does not exist and Tab never fires.
#[test]
fn every_shell_emits_a_script_for_the_scope_binary() {
    for shell in SHELLS {
        let script = script(shell);
        assert!(!script.trim().is_empty(), "{shell}: empty script");
        assert!(
            script.contains("scope"),
            "{shell}: script never names `scope`"
        );
        assert!(
            !script.contains("scope-monitor"),
            "{shell}: script targets the package name, not the binary `scope`"
        );
    }
}

/// The script is the *only* thing on stdout. `main` prints `See you later ^^`
/// when a command returns, so `completions` has to bypass that epilogue: a
/// trailing `See you later ^^` is a syntax error in every shell that sources the
/// script, i.e. an error on every prompt.
#[test]
fn script_is_the_only_thing_on_stdout() {
    for shell in SHELLS {
        let script = script(shell);
        assert!(
            !script.contains("See you later"),
            "{shell}: the `main` epilogue leaked into the completion script"
        );
    }
}

/// The script calls `scope` back through our own variable, not clap's generic
/// `COMPLETE` (see `a_stray_complete_variable_is_ignored`), and by bare name so
/// it survives the binary moving on `PATH`.
#[test]
fn every_script_calls_scope_back_through_scope_complete() {
    for shell in SHELLS {
        let script = script(shell);
        assert!(
            script.contains("SCOPE_COMPLETE"),
            "{shell}: script never sets SCOPE_COMPLETE:\n{script}"
        );
        assert!(
            !script.contains(env!("CARGO_BIN_EXE_scope")),
            "{shell}: script hard-codes the build path of the binary"
        );
    }
}

// ------------------------------------------------------------------- engine ---

/// What `scope` offers for the last of `words` (the word under the cursor,
/// possibly empty), asked the way the fish registration asks — the simplest
/// protocol: one `value<TAB>help` per line. `cwd` is where file names are
/// completed from.
fn complete_in(cwd: &Path, words: &[&str]) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .arg("--")
        .args(words)
        .env("SCOPE_COMPLETE", "fish")
        .current_dir(cwd)
        .output()
        .expect("spawn scope");
    assert!(
        out.status.success(),
        "completing {words:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("candidates are UTF-8")
        .lines()
        .map(|l| l.split('\t').next().unwrap().to_string())
        .collect()
}

fn complete(words: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    complete_in(dir.path(), words)
}

/// `scope <TAB>` offers the whole CLI surface, so a new subcommand or global
/// flag can't quietly go missing from Tab.
#[test]
fn bare_scope_offers_the_whole_cli() {
    let got = complete(&["scope", ""]);
    for want in [
        "serial",
        "rtt",
        "list",
        "ble",
        "completions",
        "--headless",
        "--capacity",
        "--tag-file",
        "--latency",
        "--name",
    ] {
        assert!(got.contains(&want.to_string()), "no {want} in {got:?}");
    }
}

/// The acceptance criterion of #236: `scope serial <TAB>` offers the live
/// ports. The test can't know what is plugged into the machine running it, so
/// it asks serialport the same question and expects the same answer — every
/// port, not only the USB ones `scope list` shows. On a machine with none (the
/// CI runners) the answer must be empty too — the path fallback is for a typed
/// word only, or an empty one would offer `.` and the working directory.
#[test]
fn serial_port_offers_the_live_ports() {
    let mut want: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect();
    want.sort();
    want.dedup();
    let mut got = complete(&["scope", "serial", ""]);
    got.retain(|c| !c.starts_with('-'));
    assert_eq!(got, want);
}

/// A port the OS doesn't enumerate — a socat or `openpty` PTY, which is how
/// scope is tested without hardware — still completes, by path.
#[cfg(unix)]
#[test]
fn serial_port_falls_back_to_paths_for_an_unlisted_pty() {
    let pty = nix::pty::openpty(None, None).expect("openpty");
    let name = nix::unistd::ttyname(&pty.slave).expect("ttyname");
    let name = name.to_str().unwrap().to_string();
    let listed = serialport::available_ports()
        .unwrap_or_default()
        .iter()
        .any(|p| p.port_name == name);
    if listed {
        return eprintln!("skip: this OS enumerates PTYs as serial ports");
    }
    let partial = &name[..name.len() - 1];
    let got = complete(&["scope", "serial", partial]);
    assert!(
        got.contains(&name),
        "{name} not offered for {partial}: {got:?}"
    );
}

#[test]
fn baudrate_offers_the_common_rates() {
    assert_eq!(
        complete(&["scope", "serial", "/dev/ttyUSB0", "11"]),
        ["115200"]
    );
    let all = complete(&["scope", "serial", "/dev/ttyUSB0", ""]);
    for want in ["9600", "115200", "921600"] {
        assert!(all.contains(&want.to_string()), "no {want} in {all:?}");
    }
}

/// `scope rtt <TAB>` offers probe-rs' chip names, matched by prefix and
/// regardless of case (probe-rs resolves them that way too).
#[test]
fn rtt_target_offers_probe_rs_chips() {
    let got = complete(&["scope", "rtt", "STM32F303V"]);
    assert!(got.contains(&"STM32F303VCTx".to_string()), "{got:?}");
    assert!(got.iter().all(|c| c.starts_with("STM32F303V")), "{got:?}");

    let lower = complete(&["scope", "rtt", "stm32f303v"]);
    assert_eq!(lower, got, "matching should ignore case");
}

#[test]
fn rtt_speed_offers_the_common_clocks() {
    let got = complete(&["scope", "rtt", "nRF52840_xxAA", "--speed", ""]);
    assert!(got.contains(&"4000".to_string()), "{got:?}");
}

/// The other half of #236's acceptance: a value that *is* a file still
/// completes to file names. The dynamic bash/fish registrations have no shell
/// file fallback of their own (bash's `-o default` is gone, fish's is
/// `--exclusive`), so this only works because the arguments carry a
/// `ValueHint` the engine completes paths from.
#[test]
fn tag_file_and_elf_complete_file_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("tags.yml"), "").unwrap();
    std::fs::write(dir.path().join("firmware.elf"), "").unwrap();
    assert_eq!(
        complete_in(dir.path(), &["scope", "--tag-file", "ta"]),
        ["tags.yml"]
    );
    assert_eq!(
        complete_in(dir.path(), &["scope", "rtt", "x", "--elf", "fi"]),
        ["firmware.elf"]
    );
}

/// Tab must work with a broken `config.toml` too: the request is answered
/// before `main` loads it. (Unix-only for the same reason as
/// `a_broken_config_file_does_not_break_the_script`.)
#[cfg(unix)]
#[test]
fn a_broken_config_file_does_not_break_tab() {
    let home = tempfile::tempdir().expect("tempdir");
    for dir in ["Library/Application Support/scope", ".config/scope"] {
        let dir = home.path().join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "capacity = \"not a number\"\n").unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["--", "scope", "se"])
        .env("SCOPE_COMPLETE", "fish")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .output()
        .expect("spawn scope");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got: Vec<&str> = stdout
        .lines()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(got, ["serial"]);
}

/// clap's default trigger variable is the generic `COMPLETE`; had we kept it,
/// anyone with `COMPLETE` set for another tool would find every `scope` run
/// hijacked into a completion request.
#[test]
fn a_stray_complete_variable_is_ignored() {
    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .arg("--version")
        .env("COMPLETE", "bash")
        .output()
        .expect("spawn scope");
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("scope "),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A shell we can't generate for is a hard error listing the ones we can, and
/// the argument is required — we never guess from `$SHELL` (which is the *login*
/// shell, so guessing would hand a zsh script to someone running bash).
#[test]
fn an_unknown_or_missing_shell_is_an_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["completions", "clamshell"])
        .output()
        .expect("spawn scope");
    assert!(!out.status.success(), "an unknown shell must be an error");
    let err = String::from_utf8_lossy(&out.stderr);
    for want in ["clamshell", "bash", "zsh", "fish", "powershell"] {
        assert!(err.contains(want), "error should mention {want}: {err}");
    }

    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .arg("completions")
        .output()
        .expect("spawn scope");
    assert!(!out.status.success(), "a missing shell must be an error");
    assert!(
        out.stdout.is_empty(),
        "a usage error must not print a half script"
    );
}

/// A broken `config.toml` is fatal for every other command — but a completion
/// script is evaluated on every shell start-up, so `completions` must not read
/// the config at all. If it did, one typo in `config.toml` would break the
/// user's prompt instead of just `scope`.
///
/// Unix-only: `dirs::config_dir()` follows `$HOME`/`$XDG_CONFIG_HOME` here, but
/// on Windows it asks the shell-known-folder API, which no env var can redirect.
#[cfg(unix)]
#[test]
fn a_broken_config_file_does_not_break_the_script() {
    let home = tempfile::tempdir().expect("tempdir");
    for dir in ["Library/Application Support/scope", ".config/scope"] {
        let dir = home.path().join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "capacity = \"not a number\"\n").unwrap();
    }

    let out = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["completions", "bash"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .output()
        .expect("spawn scope");
    assert!(
        out.status.success(),
        "completions must ignore a malformed config: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("scope"),
        "no script emitted with a malformed config present"
    );
}

// ------------------------------------------------------------------- shells ---

/// Absolute path of `prog` on `PATH`, or `None` — used to skip a shell test
/// instead of failing it when the shell isn't installed.
fn which(prog: &str) -> Option<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ""]
    } else {
        &[""]
    };
    std::env::var_os("PATH")?
        .to_str()?
        .split(sep)
        .find_map(|dir| {
            exts.iter()
                .map(|ext| Path::new(dir).join(format!("{prog}{ext}")))
                .find(|p| p.is_file())
        })
}

/// A temp dir holding the completion script for `shell` plus a copy of the
/// binary on a private `PATH` — shells only complete commands they can find.
struct Staged {
    /// Owns the staged files: dropping it deletes them, so it has to outlive the
    /// shell. Only some of the shell tests read it, and which ones are compiled
    /// depends on the platform, hence the `allow`.
    #[allow(dead_code)]
    dir: tempfile::TempDir,
    path: String,
    script: PathBuf,
}

fn stage(shell: &str, script_name: &str) -> Staged {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_scope"),
        bin.join(if cfg!(windows) { "scope.exe" } else { "scope" }),
    )
    .expect("copy scope onto PATH");

    let script_path = dir.path().join(script_name);
    std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
    std::fs::write(&script_path, script(shell)).expect("write script");

    let sep = if cfg!(windows) { ";" } else { ":" };
    let path = format!(
        "{}{sep}{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Staged {
        dir,
        path,
        script: script_path,
    }
}

/// bash completion is just a shell function, so a non-interactive bash can
/// source the script and call it the way readline does — `<command> <current>
/// <previous>` in, `COMPREPLY` out. Works on the bash 3.2 macOS still ships.
/// `words` is the line split as readline would, the last one being the word
/// under the cursor; `cwd` is where file names are completed from.
#[cfg(not(windows))]
fn bash_complete(bash: &Path, staged: &Staged, cwd: &Path, words: &[&str]) -> Vec<String> {
    // The function name is clap_complete's business: read it off the
    // `complete -F <fn> scope` line so a rename can't silently pass this test.
    let script = std::fs::read_to_string(&staged.script).unwrap();
    let func = script
        .split_whitespace()
        .skip_while(|w| *w != "-F")
        .nth(1)
        .expect("no `complete -F <fn>` line in the bash script");

    let quoted: Vec<String> = words.iter().map(|w| format!("'{w}'")).collect();
    let line = words.join(" ");
    let driver = format!(
        r#"
        source "{script}"
        COMP_LINE='{line}'; COMP_POINT={point}; COMP_TYPE=9
        COMP_WORDS=({words}); COMP_CWORD={cword}
        {func} scope {cur} {prev}
        printf '%s\n' "${{COMPREPLY[@]}}"
        "#,
        script = staged.script.display(),
        point = line.len(),
        words = quoted.join(" "),
        cword = words.len() - 1,
        cur = quoted[words.len() - 1],
        prev = quoted[words.len().saturating_sub(2)],
    );
    let out = Command::new(bash)
        .args(["--noprofile", "--norc", "-c", &driver])
        .env("PATH", &staged.path)
        .current_dir(cwd)
        .output()
        .expect("run bash");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Unix-only on purpose: the `bash` on `windows-latest` is Git-bash, whose path
/// translation makes this brittle for no gain — PowerShell covers Windows.
#[cfg(not(windows))]
#[test]
fn bash_completes_se_to_serial() {
    let Some(bash) = which("bash") else {
        return eprintln!("skip: no bash");
    };
    let staged = stage("bash", "scope.bash");
    let got = bash_complete(&bash, &staged, staged.dir.path(), &["scope", "se"]);
    assert_eq!(got, ["serial"]);
}

/// Values through a real bash: a baud rate from the binary, and a file name for
/// `--tag-file` — the case the dynamic registration's missing `-o default`
/// would otherwise break.
#[cfg(not(windows))]
#[test]
fn bash_completes_values() {
    let Some(bash) = which("bash") else {
        return eprintln!("skip: no bash");
    };
    let staged = stage("bash", "scope.bash");
    let cwd = tempfile::tempdir().expect("tempdir");
    std::fs::write(cwd.path().join("tags.yml"), "").unwrap();
    let got = bash_complete(
        &bash,
        &staged,
        cwd.path(),
        &["scope", "serial", "/dev/x", "11"],
    );
    assert_eq!(got, ["115200"]);
    let got = bash_complete(&bash, &staged, cwd.path(), &["scope", "--tag-file", "ta"]);
    assert_eq!(got, ["tags.yml"]);
}

/// fish has a first-class batch query for exactly this, and picks the script up
/// from `~/.config/fish/completions` with no config edit at all.
#[cfg(not(windows))]
fn fish_complete(fish: &Path, staged: &Staged, cwd: &Path, line: &str) -> Vec<String> {
    let out = Command::new(fish)
        .arg("--no-config")
        .arg("-c")
        .arg(format!(
            "source {}; complete -C '{line}'",
            staged.script.display()
        ))
        .env("PATH", &staged.path)
        .env("HOME", staged.dir.path())
        .current_dir(cwd)
        .output()
        .expect("run fish");
    assert!(
        out.status.success(),
        "fish failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').next().unwrap().to_string())
        .collect()
}

#[cfg(not(windows))]
#[test]
fn fish_completes_se_to_serial() {
    let Some(fish) = which("fish") else {
        return eprintln!("skip: no fish");
    };
    let staged = stage("fish", "scope.fish");
    let got = fish_complete(&fish, &staged, staged.dir.path(), "scope se");
    assert_eq!(got, ["serial"]);
}

/// fish's registration is `--exclusive`, so a file name only completes because
/// the engine offers it — pinned here alongside a value from the binary.
#[cfg(not(windows))]
#[test]
fn fish_completes_values() {
    let Some(fish) = which("fish") else {
        return eprintln!("skip: no fish");
    };
    let staged = stage("fish", "scope.fish");
    let cwd = tempfile::tempdir().expect("tempdir");
    std::fs::write(cwd.path().join("tags.yml"), "").unwrap();
    let got = fish_complete(&fish, &staged, cwd.path(), "scope serial /dev/x 11");
    assert_eq!(got, ["115200"]);
    let got = fish_complete(&fish, &staged, cwd.path(), "scope --tag-file ta");
    assert_eq!(got, ["tags.yml"]);
}

/// PowerShell exposes its completion engine directly, so no terminal is needed:
/// `TabExpansion2` returns exactly what pressing Tab would offer. This is the
/// only automated check of the Windows half of issue #231 — it runs on the
/// `windows-latest` CI job (and anywhere `pwsh` is installed).
#[test]
fn powershell_completes_se_to_serial() {
    let Some(pwsh) = which("pwsh").or_else(|| which("powershell")) else {
        return eprintln!("skip: no pwsh");
    };
    // Dot-sourcing needs a `.ps1` extension; PowerShell silently declines to run
    // any other suffix and completion then falls back to file names.
    let staged = stage("powershell", "_scope.ps1");
    let got = pwsh_complete(&pwsh, &staged, "scope se");
    assert_eq!(got, vec!["serial".to_string()], "pwsh completions wrong");
}

/// `scope <TAB>` with no partial word must list the subcommands. This is the
/// case a broken shell hook fails first (the word the shell hands over is
/// empty), and no amount of inspecting the script text catches it.
#[test]
fn powershell_bare_scope_lists_subcommands() {
    let Some(pwsh) = which("pwsh").or_else(|| which("powershell")) else {
        return eprintln!("skip: no pwsh");
    };
    let staged = stage("powershell", "_scope.ps1");
    let got = pwsh_complete(&pwsh, &staged, "scope ");
    for want in ["serial", "rtt", "list", "completions", "--headless"] {
        assert!(
            got.contains(&want.to_string()),
            "pwsh `scope <TAB>` should offer {want}, got {got:?}"
        );
    }
}

/// The Windows story of #236: PowerShell gets values from the binary too —
/// the baud list here, and COM ports from serialport's SetupAPI enumeration on
/// a real Windows box (not asserted: CI runners have none).
#[test]
fn powershell_completes_values() {
    let Some(pwsh) = which("pwsh").or_else(|| which("powershell")) else {
        return eprintln!("skip: no pwsh");
    };
    let staged = stage("powershell", "_scope.ps1");
    let got = pwsh_complete(&pwsh, &staged, "scope serial COM3 11");
    assert_eq!(got, vec!["115200".to_string()], "pwsh completions wrong");
}

/// What PowerShell would offer for `line`, with the staged script dot-sourced.
fn pwsh_complete(pwsh: &Path, staged: &Staged, line: &str) -> Vec<String> {
    let driver = format!(
        r#". "{script}"
           $l = '{line}'
           (TabExpansion2 $l $l.Length).CompletionMatches |
               ForEach-Object {{ $_.CompletionText }}"#,
        script = staged.script.display(),
    );
    let out = Command::new(pwsh)
        .args(["-NoProfile", "-NonInteractive", "-Command", &driver])
        .env("PATH", &staged.path)
        .output()
        .expect("run pwsh");
    assert!(
        out.status.success(),
        "pwsh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// zsh's completion system only runs inside a real line editor, so this is the
/// one shell that needs a PTY: spawn an interactive zsh with the script on
/// `$fpath`, type `scope se`, press Tab and read the redrawn line back.
///
/// Note the install shape being pinned here — the script must be named `_scope`
/// and `fpath` must be extended *before* `compinit`. Both orderings fail
/// silently, which is what makes this the likeliest support ticket.
#[cfg(unix)]
#[test]
fn zsh_completes_se_to_serial() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let Some(zsh) = which("zsh") else {
        return eprintln!("skip: no zsh");
    };
    let staged = stage("zsh", "zfunc/_scope");
    let rc = staged.dir.path().join("rc.zsh");
    std::fs::write(
        &rc,
        format!(
            "fpath=({zfunc} $fpath)\n\
             autoload -Uz compinit\n\
             compinit -u -d {dump}\n\
             zstyle ':completion:*' menu no\n\
             setopt no_beep\n\
             RPROMPT=''\n\
             PROMPT='ZREADY '\n",
            zfunc = staged.script.parent().unwrap().display(),
            dump = staged.dir.path().join("zcompdump").display(),
        ),
    )
    .unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(zsh);
    cmd.args(["-f", "-i"]); // -f: ignore the machine's rc files; we supply our own
    cmd.env("TERM", "xterm-256color");
    cmd.env("PATH", &staged.path);
    cmd.env("HOME", staged.dir.path());
    cmd.cwd(staged.dir.path());
    let mut child = pair.slave.spawn_command(cmd).expect("spawn zsh");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 100, 0)));
    {
        let parser = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                parser.lock().unwrap().process(&buf[..n]);
            }
        });
    }
    let screen = || parser.lock().unwrap().screen().contents();
    let wait = |needle: &str, secs: u64| {
        let start = Instant::now();
        loop {
            let s = screen();
            if s.contains(needle) {
                return s;
            }
            assert!(
                start.elapsed() < Duration::from_secs(secs),
                "timed out waiting for {needle:?}\n--- screen ---\n{s}\n---"
            );
            std::thread::sleep(Duration::from_millis(60));
        }
    };
    let mut send = |s: &str| {
        writer.write_all(s.as_bytes()).unwrap();
        writer.flush().unwrap();
    };

    send(&format!("source {}\n", rc.display()));
    wait("ZREADY", 30);
    send("scope se");
    wait("scope se", 15);
    send("\t");
    wait("scope serial", 30);

    // A value from the binary, through the same line editor (#236).
    send(" /dev/x 11\t");
    let screen = wait("scope serial /dev/x 115200", 30);

    let _ = child.kill();
    let _ = child.wait();
    assert!(
        screen.contains("scope serial /dev/x 115200"),
        "screen:\n{screen}"
    );
}
