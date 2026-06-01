use crate::se_contract::PluginSettings;
use std::path::PathBuf;

const CONFIG_FILENAME: &str = "se-ai-translator-config.json";
const CONFIG_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct ConfigFile {
    version: u32,
    settings: PluginSettings,
}

/// Get the config directory based on OS
/// Linux:   ~/.config/se-ai-translator/
/// Windows: %APPDATA%/se-ai-translator/
/// macOS:   ~/Library/Application Support/se-ai-translator/
fn get_config_dir() -> Result<PathBuf, String> {
    let base_dir = if cfg!(target_os = "windows") {
        // Windows: %APPDATA%
        dirs::config_dir().ok_or_else(|| "Cannot determine Windows config directory".to_string())?
    } else if cfg!(target_os = "macos") {
        // macOS: ~/Library/Application Support
        dirs::config_dir()
            .ok_or_else(|| "Cannot determine macOS config directory".to_string())?
            .parent()
            .ok_or_else(|| "Cannot determine macOS Application Support directory".to_string())?
            .join("Application Support")
    } else {
        // Linux and others: ~/.config
        dirs::config_dir().ok_or_else(|| "Cannot determine Linux config directory".to_string())?
    };

    let config_dir = base_dir.join("se-ai-translator");
    
    // Create directory if it doesn't exist
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Cannot create config directory: {e}"))?;

    Ok(config_dir)
}

/// Get the full path to the config file
fn get_config_path() -> Result<PathBuf, String> {
    let dir = get_config_dir()?;
    Ok(dir.join(CONFIG_FILENAME))
}

/// Load credentials from local config file
pub fn load_settings() -> Result<PluginSettings, String> {
    let path = get_config_path()?;
    
    // If file doesn't exist, return default settings
    if !path.exists() {
        crate::debug_log!("Local config file not found at {:?}, using defaults", path);
        return Ok(PluginSettings::default());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read local config: {e}"))?;
    
    let config: ConfigFile = serde_json::from_str(&content)
        .map_err(|e| format!("Cannot parse local config: {e}"))?;

    crate::debug_log!("Loaded local settings from {:?}: username={:?}", 
        path, 
        config.settings.username.as_ref().map(|_| "***")
    );

    Ok(config.settings)
}

/// Save credentials to local config file
pub fn save_settings(settings: &PluginSettings) -> Result<(), String> {
    let path = get_config_path()?;
    
    let config = ConfigFile {
        version: CONFIG_VERSION,
        settings: settings.clone(),
    };

    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Cannot serialize config: {e}"))?;
    
    std::fs::write(&path, json)
        .map_err(|e| format!("Cannot write local config: {e}"))?;

    crate::debug_log!("Saved local settings to {:?}: username={:?}, has_token={}", 
        path, 
        settings.username.as_ref().map(|_| "***"),
        settings.auth_token.is_some()
    );

    Ok(())
}

/// Clear all saved credentials
#[allow(dead_code)]
pub fn clear_settings() -> Result<(), String> {
    let path = get_config_path()?;
    
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Cannot delete config file: {e}"))?;
        crate::debug_log!("Cleared local settings from {:?}", path);
    }

    Ok(())
}
