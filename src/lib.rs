use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(rename = "var", default)]
    pub variables: Vec<Variable>,
    #[serde(rename = "game", default)]
    pub games: Vec<Game>,
}

#[derive(Debug, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct Game {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(rename = "save", default)]
    pub saves: Vec<SaveLocation>,
}

#[derive(Debug, Deserialize)]
pub struct SaveLocation {
    pub path: String,
    #[serde(default)]
    pub files: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

pub struct GameBackup {
    config: Config,
    variables: HashMap<String, String>,
    backup_root: PathBuf,
}

impl GameBackup {
    pub fn new(config_path: &Path) -> Result<Self> {
        log::info!("Loading configuration from: {}", config_path.display());

        let config_content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

        log::debug!("Parsing TOML configuration");
        let config: Config = toml::from_str(&config_content)
            .with_context(|| "Failed to parse TOML configuration")?;

        log::info!(
            "Successfully loaded {} games and {} variables",
            config.games.len(),
            config.variables.len()
        );

        let backup_root = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backup");

        log::info!("Backup root directory: {}", backup_root.display());

        let mut game_backup = Self {
            config,
            variables: HashMap::new(),
            backup_root,
        };

        game_backup.resolve_variables()?;
        Ok(game_backup)
    }

    fn resolve_variables(&mut self) -> Result<()> {
        log::info!("Resolving variables");

        // Add built-in system variables
        self.add_system_variables()?;

        // Check for reserved variable names
        for var in &self.config.variables {
            if var.name == "home" {
                return Err(anyhow!(
                    "Variable name 'home' is reserved and cannot be used in configuration"
                ));
            } else if var.name == "config" {
                return Err(anyhow!(
                    "Variable name 'config' is reserved and cannot be used in configuration"
                ));
            }
        }

        // Resolve user-defined variables in order (top to bottom)
        for var in &self.config.variables {
            log::debug!("Resolving variable: {} = {}", var.name, var.value);
            let resolved_value = self.expand_variables(&var.value)?;
            self.variables.insert(var.name.clone(), resolved_value);
            log::debug!(
                "Variable '{}' resolved to: {}",
                var.name,
                self.variables[&var.name]
            );
        }

        log::info!("Successfully resolved {} variables", self.variables.len());
        Ok(())
    }

    fn add_system_variables(&mut self) -> Result<()> {
        log::debug!("Adding system variables");

        #[cfg(windows)]
        {
            if let Some(home_dir) = dirs::home_dir() {
                self.variables
                    .insert("home".to_string(), home_dir.to_string_lossy().to_string());
                log::debug!("Added system variable 'home': {}", home_dir.display());
            } else {
                log::warn!("Could not determine home directory");
            }
            if let Some(appdata) = dirs::config_dir() {
                self.variables
                    .insert("config".to_string(), appdata.to_string_lossy().to_string());
                log::debug!("Added system variable 'config': {}", appdata.display());
            } else {
                log::warn!("Could not determine config directory");
            }
        }

        #[cfg(unix)]
        {
            if let Some(home_dir) = dirs::home_dir() {
                self.variables
                    .insert("home".to_string(), home_dir.to_string_lossy().to_string());
                log::debug!("Added system variable 'home': {}", home_dir.display());
            } else {
                log::warn!("Could not determine home directory");
            }
            if let Some(config_dir) = dirs::config_dir() {
                self.variables.insert(
                    "config".to_string(),
                    config_dir.to_string_lossy().to_string(),
                );
                log::debug!("Added system variable 'config': {}", config_dir.display());
            } else {
                log::warn!("Could not determine config directory");
            }
        }

        Ok(())
    }

    fn expand_variables(&self, value: &str) -> Result<String> {
        let mut result = value.to_string();
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 10;

        while result.contains("${") && iterations < MAX_ITERATIONS {
            let mut changed = false;
            let mut new_result = String::new();
            let mut chars = result.chars().peekable();

            while let Some(ch) = chars.next() {
                if ch == '$' && chars.peek() == Some(&'{') {
                    chars.next(); // consume '{'
                    let mut var_name = String::new();

                    while let Some(ch) = chars.next() {
                        if ch == '}' {
                            break;
                        }
                        var_name.push(ch);
                    }

                    if let Some(var_value) = self.variables.get(&var_name) {
                        new_result.push_str(var_value);
                        changed = true;
                    } else {
                        return Err(anyhow!("Undefined variable: {}", var_name));
                    }
                } else {
                    new_result.push(ch);
                }
            }

            result = new_result;
            iterations += 1;

            if !changed {
                break;
            }
        }

        if iterations >= MAX_ITERATIONS {
            return Err(anyhow!(
                "Variable resolution exceeded maximum iterations (possible circular reference)"
            ));
        }

        Ok(result)
    }

    pub fn list_games(&self) -> Vec<&Game> {
        log::info!("Listing games from configuration");
        let enabled_games: Vec<&Game> = self
            .config
            .games
            .iter()
            .filter(|game| game.enabled)
            .collect();

        log::info!("Found {} enabled games", enabled_games.len());
        enabled_games
    }

    pub fn has_backup(&self, game_name: &str) -> bool {
        let game_backup_dir = self.backup_root.join(game_name);
        let has_backup = game_backup_dir.exists();
        log::debug!("Checking backup for '{}': {}", game_name, has_backup);
        has_backup
    }

    pub fn backup_game(&self, game_name: &str) -> Result<()> {
        log::info!("Starting backup for game: {}", game_name);

        let game = self
            .config
            .games
            .iter()
            .find(|g| g.name == game_name)
            .ok_or_else(|| anyhow!("Game '{}' not found in configuration", game_name))?;

        if !game.enabled {
            log::warn!("Game '{}' is disabled, skipping backup", game_name);
            return Ok(());
        }

        let game_backup_dir = self.backup_root.join(&game.name);
        log::info!("Creating backup directory: {}", game_backup_dir.display());
        fs::create_dir_all(&game_backup_dir).with_context(|| {
            format!(
                "Failed to create backup directory: {}",
                game_backup_dir.display()
            )
        })?;

        for (i, save_location) in game.saves.iter().enumerate() {
            log::info!(
                "Processing save location {}/{} for game '{}'",
                i + 1,
                game.saves.len(),
                game.name
            );
            self.backup_save_location(save_location, &game_backup_dir)?;
        }

        log::info!("Successfully completed backup for game: {}", game_name);
        Ok(())
    }

    fn backup_save_location(
        &self,
        save_location: &SaveLocation,
        game_backup_dir: &Path,
    ) -> Result<()> {
        let source_path = self.expand_variables(&save_location.path)?;
        let source_path = Path::new(&source_path);

        log::info!("Backing up from: {}", source_path.display());

        if !source_path.exists() {
            return Err(anyhow!(
                "Save path does not exist: {}",
                source_path.display()
            ));
        }

        let backup_subdir = self.create_backup_path(source_path, game_backup_dir)?;
        log::debug!("Backup destination: {}", backup_subdir.display());

        fs::create_dir_all(&backup_subdir).with_context(|| {
            format!(
                "Failed to create backup subdirectory: {}",
                backup_subdir.display()
            )
        })?;

        if save_location.files.is_empty() {
            log::info!("No specific files specified, backing up all files recursively");
            self.copy_all_files(source_path, &backup_subdir)?;
        } else {
            log::info!(
                "Backing up {} specific file patterns",
                save_location.files.len()
            );
            for pattern in &save_location.files {
                self.copy_files_by_pattern(source_path, &backup_subdir, pattern)?;
            }
        }

        Ok(())
    }

    fn create_backup_path(&self, source_path: &Path, game_backup_dir: &Path) -> Result<PathBuf> {
        let mut backup_path = game_backup_dir.to_path_buf();

        #[cfg(windows)]
        {
            if let Some(prefix) = source_path.components().next() {
                if let std::path::Component::Prefix(prefix_component) = prefix {
                    if let std::path::Prefix::Disk(drive_letter) = prefix_component.kind() {
                        let drive_name =
                            format!("drive_{}", (drive_letter as char).to_ascii_lowercase());
                        backup_path.push(drive_name);

                        // Process the rest of the path components
                        let remaining_components: Vec<_> =
                            source_path.components().skip(1).collect();
                        let anonymized_path = self.anonymize_windows_path(&remaining_components)?;

                        for component in anonymized_path.components() {
                            if let std::path::Component::Normal(name) = component {
                                backup_path.push(name);
                            }
                        }
                    }
                }
            }
        }

        #[cfg(unix)]
        {
            let anonymized_path = self.anonymize_unix_path(source_path)?;
            for component in anonymized_path.components() {
                if let std::path::Component::Normal(name) = component {
                    backup_path.push(name);
                }
            }
        }

        Ok(backup_path)
    }

    #[cfg(windows)]
    fn anonymize_windows_path(&self, components: &[std::path::Component]) -> Result<PathBuf> {
        let mut result = PathBuf::new();
        let mut i = 0;

        while i < components.len() {
            match &components[i] {
                std::path::Component::Normal(name) => {
                    let name_str = name.to_string_lossy();

                    // Check if we're at Users/[username] pattern
                    if name_str.eq_ignore_ascii_case("Users") && i + 1 < components.len() {
                        if let std::path::Component::Normal(_username) = &components[i + 1] {
                            // Replace Users/[username] with Users/user_home
                            result.push("Users");
                            result.push("user_home");
                            i += 2; // Skip both Users and username components
                            continue;
                        }
                    }

                    // Regular component, add as-is
                    result.push(name);
                    i += 1;
                }
                _ => {
                    // Should not happen in the remaining components, but handle gracefully
                    i += 1;
                }
            }
        }

        Ok(result)
    }

    #[cfg(unix)]
    fn anonymize_unix_path(&self, path: &Path) -> Result<PathBuf> {
        if let Some(home_dir) = dirs::home_dir() {
            if let Ok(relative_path) = path.strip_prefix(&home_dir) {
                // Path is under home directory, replace with user_home
                let mut anonymized = PathBuf::from("user_home");
                anonymized.push(relative_path);
                return Ok(anonymized);
            }
        }

        // Path is not under home directory, keep as is but remove leading slash
        if path.is_absolute() {
            let mut result = PathBuf::new();
            for component in path.components().skip(1) {
                // Skip root component
                if let std::path::Component::Normal(name) = component {
                    result.push(name);
                }
            }
            return Ok(result);
        }

        Ok(path.to_path_buf())
    }

    fn copy_all_files(&self, source: &Path, dest: &Path) -> Result<()> {
        log::debug!(
            "Copying all files from {} to {}",
            source.display(),
            dest.display()
        );

        if source.is_file() {
            let file_name = source
                .file_name()
                .ok_or_else(|| anyhow!("Invalid file name: {}", source.display()))?;
            let dest_file = dest.join(file_name);
            log::debug!(
                "Copying file: {} -> {}",
                source.display(),
                dest_file.display()
            );
            fs::copy(source, dest_file)
                .with_context(|| format!("Failed to copy file: {}", source.display()))?;
            return Ok(());
        }

        let entries = fs::read_dir(source)
            .with_context(|| format!("Failed to read directory: {}", source.display()))?;

        for entry in entries {
            let entry = entry.with_context(|| {
                format!("Failed to read directory entry in: {}", source.display())
            })?;
            let path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest.join(&file_name);

            if path.is_dir() {
                log::debug!("Creating directory: {}", dest_path.display());
                fs::create_dir_all(&dest_path).with_context(|| {
                    format!("Failed to create directory: {}", dest_path.display())
                })?;
                self.copy_all_files(&path, &dest_path)?;
            } else {
                log::debug!(
                    "Copying file: {} -> {}",
                    path.display(),
                    dest_path.display()
                );
                fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy file: {}", path.display()))?;
            }
        }

        Ok(())
    }

    fn copy_files_by_pattern(
        &self,
        source_dir: &Path,
        dest_dir: &Path,
        pattern: &str,
    ) -> Result<()> {
        let full_pattern = source_dir.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        log::debug!("Searching for files matching pattern: {}", pattern_str);

        let paths = glob::glob(&pattern_str)
            .with_context(|| format!("Invalid glob pattern: {}", pattern_str))?;

        let mut file_count = 0;
        for path_result in paths {
            let path = path_result
                .with_context(|| format!("Error processing glob pattern: {}", pattern_str))?;

            if path.is_file() {
                let file_name = path
                    .file_name()
                    .ok_or_else(|| anyhow!("Invalid file name: {}", path.display()))?;
                let dest_file = dest_dir.join(file_name);

                log::debug!(
                    "Copying file: {} -> {}",
                    path.display(),
                    dest_file.display()
                );
                fs::copy(&path, &dest_file)
                    .with_context(|| format!("Failed to copy file: {}", path.display()))?;
                file_count += 1;
            }
        }

        log::info!("Copied {} files matching pattern: {}", file_count, pattern);
        Ok(())
    }

    pub fn restore_game(&self, game_name: &str) -> Result<()> {
        log::info!("Starting restore for game: {}", game_name);

        let game = self
            .config
            .games
            .iter()
            .find(|g| g.name == game_name)
            .ok_or_else(|| anyhow!("Game '{}' not found in configuration", game_name))?;

        if !game.enabled {
            log::warn!("Game '{}' is disabled, skipping restore", game_name);
            return Ok(());
        }

        let game_backup_dir = self.backup_root.join(&game.name);
        if !game_backup_dir.exists() {
            return Err(anyhow!("No backup found for game: {}", game_name));
        }

        for (i, save_location) in game.saves.iter().enumerate() {
            log::info!(
                "Processing restore location {}/{} for game '{}'",
                i + 1,
                game.saves.len(),
                game.name
            );
            self.restore_save_location(save_location, &game_backup_dir)?;
        }

        log::info!("Successfully completed restore for game: {}", game_name);
        Ok(())
    }

    fn restore_save_location(
        &self,
        save_location: &SaveLocation,
        game_backup_dir: &Path,
    ) -> Result<()> {
        let dest_path = self.expand_variables(&save_location.path)?;
        let dest_path = Path::new(&dest_path);

        log::info!("Restoring to: {}", dest_path.display());

        let backup_subdir = self.create_backup_path(dest_path, game_backup_dir)?;
        log::debug!("Restore source: {}", backup_subdir.display());

        if !backup_subdir.exists() {
            return Err(anyhow!(
                "Backup directory does not exist: {}",
                backup_subdir.display()
            ));
        }

        // Create destination directory if it doesn't exist
        fs::create_dir_all(dest_path).with_context(|| {
            format!(
                "Failed to create destination directory: {}",
                dest_path.display()
            )
        })?;

        self.copy_all_files(&backup_subdir, dest_path)?;
        Ok(())
    }

    pub fn backup_all_games(&self) -> Result<()> {
        log::info!("Starting backup for all enabled games");

        let enabled_games: Vec<&Game> = self
            .config
            .games
            .iter()
            .filter(|game| game.enabled)
            .collect();

        if enabled_games.is_empty() {
            log::warn!("No enabled games found in configuration");
            return Ok(());
        }

        let mut success_count = 0;
        let mut error_count = 0;

        for game in enabled_games {
            match self.backup_game(&game.name) {
                Ok(()) => {
                    success_count += 1;
                    log::info!("✓ Successfully backed up: {}", game.name);
                }
                Err(e) => {
                    error_count += 1;
                    log::error!("✗ Failed to backup '{}': {}", game.name, e);
                }
            }
        }

        log::info!(
            "Backup summary: {} successful, {} failed",
            success_count,
            error_count
        );

        if error_count > 0 {
            return Err(anyhow!(
                "Some backups failed. Check the logs above for details."
            ));
        }

        Ok(())
    }

    pub fn restore_all_games(&self) -> Result<()> {
        log::info!("Starting restore for all enabled games");

        let enabled_games: Vec<&Game> = self
            .config
            .games
            .iter()
            .filter(|game| game.enabled)
            .collect();

        if enabled_games.is_empty() {
            log::warn!("No enabled games found in configuration");
            return Ok(());
        }

        let mut success_count = 0;
        let mut error_count = 0;

        for game in enabled_games {
            match self.restore_game(&game.name) {
                Ok(()) => {
                    success_count += 1;
                    log::info!("✓ Successfully restored: {}", game.name);
                }
                Err(e) => {
                    error_count += 1;
                    log::error!("✗ Failed to restore '{}': {}", game.name, e);
                }
            }
        }

        log::info!(
            "Restore summary: {} successful, {} failed",
            success_count,
            error_count
        );

        if error_count > 0 {
            return Err(anyhow!(
                "Some restores failed. Check the logs above for details."
            ));
        }

        Ok(())
    }
}

pub fn find_config_file(config_path: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = config_path {
        let config_path = PathBuf::from(path);
        if config_path.exists() {
            log::info!("Using specified config file: {}", config_path.display());
            return Ok(config_path);
        } else {
            return Err(anyhow!(
                "Specified config file not found: {}",
                config_path.display()
            ));
        }
    }

    // Look for TOML files in current directory
    log::info!("No config file specified, searching for TOML files in current directory");
    let current_dir = std::env::current_dir().with_context(|| "Failed to get current directory")?;

    let entries = fs::read_dir(&current_dir).with_context(|| {
        format!(
            "Failed to read current directory: {}",
            current_dir.display()
        )
    })?;

    let mut toml_files = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let path = entry.path();

        if path.is_file() {
            if let Some(extension) = path.extension() {
                if extension == "toml" {
                    toml_files.push(path);
                }
            }
        }
    }

    match toml_files.len() {
        0 => Err(anyhow!(
            "No TOML configuration files found in current directory"
        )),
        1 => {
            log::info!("Found config file: {}", toml_files[0].display());
            Ok(toml_files[0].clone())
        }
        _ => {
            let file_names: Vec<String> = toml_files
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect();
            Err(anyhow!(
                "Multiple TOML files found in current directory: {}. Please specify which one to use with --config",
                file_names.join(", ")
            ))
        }
    }
}
