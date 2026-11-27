//! Which plugin and which `test_*` functions a `!test` command runs.

use std::sync::Arc;

pub const TEST_PREFIX: &str = "test_";

const USAGE: &str = "Usage: !test <plugin> [test ...] or !test <test> [test ...]";

#[derive(Debug, PartialEq)]
pub struct Selection {
    pub plugin: Arc<String>,
    /// Full function names, in the order they run.
    pub tests: Vec<String>,
}

/// Resolves the `!test` arguments against the loaded plugins and their tests.
///
/// A first argument naming a loaded plugin selects that plugin (all its tests
/// when no test follows); otherwise every argument is a test name, which must
/// be defined by exactly one plugin. Test names may omit the `test_` prefix.
pub fn select(
    args: &[String],
    catalog: &[(Arc<String>, Vec<String>)],
) -> Result<Selection, String> {
    let Some(first) = args.first() else {
        return Err(USAGE.to_string());
    };

    if let Some((plugin, tests)) = catalog.iter().find(|(name, _)| name.as_str() == first) {
        return select_in_plugin(plugin, tests, &args[1..]);
    }

    let mut plugin: Option<&Arc<String>> = None;
    let mut selected = vec![];

    for arg in args {
        let test = with_prefix(arg);
        let mut owners = catalog
            .iter()
            .filter(|(_, tests)| tests.contains(&test))
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        owners.sort();

        let owner = match owners.as_slice() {
            [] => return Err(format!("No loaded plugin has a test \"{arg}\"")),
            [owner] => *owner,
            _ => {
                return Err(format!(
                    "Test \"{arg}\" is defined by {}; use !test <plugin> {arg}",
                    join(&owners)
                ));
            }
        };

        match plugin {
            Some(plugin) if plugin != owner => {
                return Err(format!(
                    "The tests belong to different plugins ({}, {}); run one plugin at a time",
                    plugin, owner
                ));
            }
            _ => plugin = Some(owner),
        }

        push_unique(&mut selected, test);
    }

    Ok(Selection {
        plugin: plugin.expect("args is not empty").clone(),
        tests: selected,
    })
}

fn select_in_plugin(
    plugin: &Arc<String>,
    tests: &[String],
    args: &[String],
) -> Result<Selection, String> {
    if tests.is_empty() {
        return Err(format!(
            "Plugin \"{plugin}\" has no tests (functions named {TEST_PREFIX}*)"
        ));
    }

    if args.is_empty() {
        return Ok(Selection {
            plugin: plugin.clone(),
            tests: tests.to_vec(),
        });
    }

    let mut selected = vec![];
    for arg in args {
        let test = with_prefix(arg);
        if !tests.contains(&test) {
            return Err(format!("Plugin \"{plugin}\" has no test \"{arg}\""));
        }
        push_unique(&mut selected, test);
    }

    Ok(Selection {
        plugin: plugin.clone(),
        tests: selected,
    })
}

fn with_prefix(name: &str) -> String {
    if name.starts_with(TEST_PREFIX) {
        name.to_string()
    } else {
        format!("{TEST_PREFIX}{name}")
    }
}

fn push_unique(selected: &mut Vec<String>, test: String) {
    if !selected.contains(&test) {
        selected.push(test);
    }
}

fn join(names: &[&Arc<String>]) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(entries: &[(&str, &[&str])]) -> Vec<(Arc<String>, Vec<String>)> {
        entries
            .iter()
            .map(|(name, tests)| {
                (
                    Arc::new(name.to_string()),
                    tests.iter().map(|t| t.to_string()).collect(),
                )
            })
            .collect()
    }

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    fn selection(plugin: &str, tests: &[&str]) -> Result<Selection, String> {
        Ok(Selection {
            plugin: Arc::new(plugin.to_string()),
            tests: args(tests),
        })
    }

    #[test]
    fn a_plugin_name_runs_its_whole_suite_in_order() {
        let catalog = catalog(&[("at", &["test_at", "test_cops"])]);

        assert_eq!(
            select(&args(&["at"]), &catalog),
            selection("at", &["test_at", "test_cops"])
        );
    }

    #[test]
    fn a_plugin_name_followed_by_tests_runs_only_those() {
        let catalog = catalog(&[("at", &["test_at", "test_cops", "test_csq"])]);

        assert_eq!(
            select(&args(&["at", "csq", "test_at"]), &catalog),
            selection("at", &["test_csq", "test_at"])
        );
    }

    #[test]
    fn a_test_name_finds_its_plugin_with_or_without_the_prefix() {
        let catalog = catalog(&[("at", &["test_cops"]), ("echo", &[])]);

        assert_eq!(
            select(&args(&["cops"]), &catalog),
            selection("at", &["test_cops"])
        );
        assert_eq!(
            select(&args(&["test_cops"]), &catalog),
            selection("at", &["test_cops"])
        );
    }

    #[test]
    fn a_loaded_plugin_name_takes_precedence_over_a_test_name() {
        let catalog = catalog(&[("cops", &["test_x"]), ("at", &["test_cops"])]);

        assert_eq!(
            select(&args(&["cops"]), &catalog),
            selection("cops", &["test_x"])
        );
    }

    #[test]
    fn a_test_defined_by_two_plugins_is_ambiguous() {
        let catalog = catalog(&[("quectel", &["test_cops"]), ("at", &["test_cops"])]);

        assert_eq!(
            select(&args(&["cops"]), &catalog),
            Err("Test \"cops\" is defined by at, quectel; use !test <plugin> cops".to_string())
        );
    }

    #[test]
    fn tests_of_different_plugins_are_rejected() {
        let catalog = catalog(&[("at", &["test_cops"]), ("gps", &["test_fix"])]);

        assert!(
            select(&args(&["cops", "fix"]), &catalog)
                .unwrap_err()
                .contains("different plugins")
        );
    }

    #[test]
    fn an_unknown_test_is_reported() {
        let catalog = catalog(&[("at", &["test_cops"])]);

        assert_eq!(
            select(&args(&["at", "csq"]), &catalog),
            Err("Plugin \"at\" has no test \"csq\"".to_string())
        );
        assert_eq!(
            select(&args(&["csq"]), &catalog),
            Err("No loaded plugin has a test \"csq\"".to_string())
        );
    }

    #[test]
    fn a_plugin_without_tests_is_reported() {
        let catalog = catalog(&[("echo", &[])]);

        assert!(
            select(&args(&["echo"]), &catalog)
                .unwrap_err()
                .contains("has no tests")
        );
    }

    #[test]
    fn no_argument_prints_the_usage() {
        assert_eq!(select(&[], &[]), Err(USAGE.to_string()));
    }

    #[test]
    fn a_repeated_test_runs_once() {
        let catalog = catalog(&[("at", &["test_cops"])]);

        assert_eq!(
            select(&args(&["cops", "test_cops"]), &catalog),
            selection("at", &["test_cops"])
        );
    }
}
