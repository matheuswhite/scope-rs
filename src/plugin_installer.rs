use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

pub struct PluginInstaller;

macro_rules! wrn {
    ($content:expr) => {
        println!("[\x1b[33mWRN\x1b[0m] {}", $content);
    };
}

macro_rules! ok {
    ($content:expr) => {
        println!("[\x1b[32mOK\x1b[0m] {}", $content);
    };
}

impl PluginInstaller {
    const SCOPE: &'static str = include_str!("../plugins/scope.lua");

    fn get_hash(&self, content: &str) -> u64 {
        let mut hasher = DefaultHasher::default();
        content.hash(&mut hasher);
        hasher.finish()
    }

    fn create_dir(&self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            wrn!(format!("Creating {:?}...", path));
            std::fs::create_dir(path).map_err(|_| format!("Cannot create {:?}", path))?;
            ok!(format!("{:?} created with success", path));
        } else {
            ok!(format!("{:?} is green", path));
        }

        Ok(())
    }

    fn create_file(&self, path: &Path, content: &str) -> Result<(), String> {
        if !path.exists() {
            wrn!(format!("Creating {:?}...", path));
            std::fs::write(path, content)
                .map_err(|_| format!("Cannot create {:?} into plugins directory", path))?;
            ok!(format!("{:?} created with success", path));
        } else {
            ok!(format!("{:?} is green", path));
        }

        Ok(())
    }

    pub fn post(&self) -> Result<(), String> {
        let mut dir = homedir::get_my_home()
            .map_err(|_| "Cannot get home dir".to_string())?
            .ok_or("Home dir path is empty".to_string())?;

        dir.push(".config/");
        self.create_dir(&dir)?;

        dir.push("scope/");
        self.create_dir(&dir)?;

        dir.push("plugins/");
        self.create_dir(&dir)?;

        dir.push("scope.lua");
        self.create_file(&dir, PluginInstaller::SCOPE)?;

        wrn!(format!("Checking {:?} integrity", dir));
        let right_hash = self.get_hash(PluginInstaller::SCOPE);
        let read_hash = self.get_hash(
            std::fs::read_to_string(&dir)
                .map_err(|_| "Cannot read scope.lua content".to_string())?
                .as_str(),
        );

        if right_hash != read_hash {
            wrn!(format!(
                "{:?} is corrupted. Replacing by original content...",
                &dir
            ));
            std::fs::write(&dir, PluginInstaller::SCOPE).map_err(|_| {
                "Cannot replace original content of scope.lua into file".to_string()
            })?;
        }

        ok!(format!("{:?} is green", &dir));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin::Plugin;
    use crate::plugin_installer::PluginInstaller;

    #[test]
    fn test_post_only_config() {
        let mut config_dir = homedir::get_my_home().unwrap().unwrap();
        config_dir.push(".config/scope");

        std::fs::remove_dir_all(&config_dir).unwrap();

        PluginInstaller.post().unwrap();
        Plugin::new("plugins/test.lua".into()).unwrap();
    }

    #[test]
    fn test_post_only_scope() {
        let mut config_dir = homedir::get_my_home().unwrap().unwrap();
        config_dir.push(".config/scope/plugins");

        std::fs::remove_dir_all(&config_dir).unwrap();

        PluginInstaller.post().unwrap();
        Plugin::new("plugins/test.lua".into()).unwrap();
    }

    #[test]
    fn test_post_only_plugins_folder() {
        let mut config_dir = homedir::get_my_home().unwrap().unwrap();
        config_dir.push(".config/scope/plugins/scope.lua");

        std::fs::remove_file(&config_dir).unwrap();

        PluginInstaller.post().unwrap();
        Plugin::new("plugins/test.lua".into()).unwrap();
    }
}
