//! Command line argument parsing.

use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use toml::Value;

use crate::config::Config;

/// Command line arguments.
#[derive(Parser, Debug)]
#[clap(author, about, version, max_term_width = 80)]
pub struct Options {
    /// Links which will be opened at startup.
    #[clap(allow_hyphen_values = true, num_args = 1..)]
    pub links: Vec<String>,

    #[clap(subcommand)]
    pub subcommands: Option<Subcommands>,
}

#[derive(Subcommand, Debug)]
pub enum Subcommands {
    /// Change configuration through IPC.
    #[clap(subcommand)]
    Config(ConfigOptions),
}

#[derive(Subcommand, Debug)]
pub enum ConfigOptions {
    /// Get the current value of an option
    Get(GetConfig),
    /// Set a runtime override for an option
    Set(SetConfig),
    /// Reset an option to its config file value
    Reset(ResetConfig),
}

#[derive(Args, Debug)]
pub struct GetConfig {
    /// Config option path [example: 'colors.fg'].
    #[clap(value_name = "PATH")]
    pub path: Option<String>,
}

#[derive(Args, Debug)]
pub struct SetConfig {
    /// Config option path [example: 'colors.fg'].
    #[clap(required = true, value_name = "PATH")]
    pub path: String,

    /// New config value.
    #[clap(required = true, value_name = "VALUE")]
    pub value: String,
}

#[derive(Args, Debug)]
pub struct ResetConfig {
    /// Config option path [example: 'colors.fg'].
    #[clap(required = true, value_name = "PATH")]
    pub path: String,
}

/// Try to convert a CLI argument string to a TOML value.
pub fn parse_toml_value(path: &str, value: String) -> Result<Value, toml::de::Error> {
    // Attempt to deserialize as a full config, to catch type errors.

    // Deserialize to generic value, to ensure syntax is correct.
    let mut value = match toml::from_str::<Value>(&format!("{path} = {value}")) {
        Ok(value) => value,
        // Add quotes to value to work around common string type issues.
        Err(err) => match toml::from_str::<Value>(&format!("{path} = \"{value}\"")) {
            Ok(value) => value,
            Err(_) => return Err(err),
        },
    };

    // Attempt to parse as config, to ensure types are correct.
    Config::deserialize(value.clone())?;

    // Resolve path in the raw toml tree, to get the parsed value.
    for segment in path.split('.') {
        match value {
            Value::Table(mut table) => {
                value = table.remove(segment).unwrap();
            },
            _ => unreachable!(),
        }
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use toml::Table;

    use super::*;

    #[test]
    fn parse_toml_simple() {
        let parsed = parse_toml_value("font.family", "\"huh\"".into());
        assert_eq!(parsed, Ok(Value::String("huh".into())));

        let parsed = parse_toml_value("font.family", "huh".into());
        assert_eq!(parsed, Ok(Value::String("huh".into())));

        let parsed = parse_toml_value("font.size", "1.0".into());
        assert_eq!(parsed, Ok(Value::Float(1.0)));
    }

    #[test]
    fn parse_toml_color() {
        let parsed = parse_toml_value("colors.fg", "\"#ff00ff\"".into());
        assert_eq!(parsed, Ok(Value::String("#ff00ff".into())));

        let parsed = parse_toml_value("colors.fg", "#00ff00".into());
        assert_eq!(parsed, Ok(Value::String("#00ff00".into())));
    }

    #[test]
    fn parse_toml_table() {
        let mut table = Table::new();
        table.insert("fg".into(), Value::String("#ff00ff".into()));
        table.insert("bg".into(), Value::String("#00ff00".into()));
        let expected = Value::Table(table);

        let parsed = parse_toml_value("colors", "{ fg = \"#ff00ff\", bg = \"#00ff00\" }".into());

        assert_eq!(parsed, Ok(expected));
    }

    #[test]
    fn parse_toml_invalid_syntax() {
        let parsed = parse_toml_value("colors[[[", "blub".into());
        assert!(parsed.is_err());
    }

    #[test]
    fn parse_toml_invalid_type() {
        let parsed = parse_toml_value("colors.fg", "13".into());
        assert!(parsed.is_err());
    }

    #[test]
    fn parse_toml_invalid_path() {
        let parsed = parse_toml_value("doesnotexist", "13".into());
        assert!(parsed.is_err());
    }
}
