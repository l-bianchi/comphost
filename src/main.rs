use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::Command;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add new configurations
    Add {
        #[arg(value_name = "NAME")]
        name: Vec<String>,
    },
    /// Turn on configurations
    On {
        #[arg(value_name = "NAME")]
        name: Vec<String>,
    },
    /// Turn off configurations
    Off {
        #[arg(value_name = "NAME")]
        name: Vec<String>,
    },
    /// Clone active configurations
    Clone,
    /// Start Docker Compose for active configurations
    Start,
    /// Stop Docker Compose for active configurations
    Stop,
    /// List configuration names for shell completion
    ListNames,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Configuration {
    active: bool,
    url: String,
    clone_path: Option<String>,
}

impl Configuration {
    fn clone_project(&mut self, clone_path: String) {
        self.clone_path = Some(clone_path);
    }
}

fn main() {
    let args = Cli::parse();

    // Get the user's home directory
    let home_dir = match env::var("HOME") {
        Ok(val) => val,
        Err(_) => {
            eprintln!("Failed to get user's home directory");
            return;
        }
    };

    // Define the config directory and file path
    let config_dir = format!("{}/.config/comphost", home_dir);
    let config_file_path = format!("{}/config.toml", config_dir);

    // Ensure the config directory exists
    if let Err(err) = std::fs::create_dir_all(&config_dir) {
        eprintln!("Failed to create config directory: {}", err);
        return;
    }

    // Read the existing TOML content if the file exists
    let mut existing_content = String::new();
    if let Ok(mut file) = File::open(&config_file_path) {
        file.read_to_string(&mut existing_content)
            .expect("Could not read file");
    }

    // Deserialize existing TOML content into a HashMap
    let mut toml_content: HashMap<String, Configuration> = if existing_content.is_empty() {
        HashMap::new()
    } else {
        toml::from_str(&existing_content).expect("Could not parse TOML")
    };

    match args.command {
        Commands::Add { name } => {
            for config_name in &name {
                // Prompt the user for a URL
                println!("Enter URL for '{}':", config_name);
                let mut url = String::new();
                io::stdin()
                    .read_line(&mut url)
                    .expect("Failed to read line");

                // Add or update the new configuration
                let config = Configuration {
                    active: true,
                    url: url.trim().to_string(),
                    clone_path: None,
                };
                toml_content.insert(config_name.clone(), config);
                println!("Configuration '{}' added.", config_name);
            }
        }
        Commands::On { name } => {
            for config_name in &name {
                if let Some(config) = toml_content.get_mut(config_name) {
                    config.active = true;
                    println!("Configuration '{}' turned on.", config_name);
                } else {
                    eprintln!("Configuration '{}' not found.", config_name);
                }
            }
        }
        Commands::Off { name } => {
            for config_name in &name {
                if let Some(config) = toml_content.get_mut(config_name) {
                    config.active = false;
                    println!("Configuration '{}' turned off.", config_name);
                } else {
                    eprintln!("Configuration '{}' not found.", config_name);
                }
            }
        }
        Commands::Clone => {
            println!("Enter the path where you want to clone:");
            let mut clone_dir = String::new();
            io::stdin()
                .read_line(&mut clone_dir)
                .expect("Failed to read line");

            let clone_dir = clone_dir.trim();
            for (config_name, config) in &mut toml_content {
                if config.active {
                    let clone_path = format!("{}/{}", clone_dir, config_name);
                    if let Ok(metadata) = std::fs::metadata(&clone_path) {
                        if metadata.is_dir() {
                            println!(
                                "Skipping '{}', folder already exists at '{}'",
                                config_name, clone_path
                            );
                            config.clone_project(clone_path);
                            continue;
                        } else {
                            eprintln!("Path '{}' exists but is not a directory", clone_path);
                            continue;
                        }
                    }

                    let clone_command = Command::new("git")
                        .arg("clone")
                        .arg(&config.url)
                        .arg(config_name)
                        .current_dir(clone_dir)
                        .output()
                        .expect("Failed to execute git clone command");

                    if clone_command.status.success() {
                        println!(
                            "Cloned '{}' from '{}' to '{}'",
                            config_name, config.url, clone_path
                        );
                        config.clone_project(clone_path);
                    } else {
                        eprintln!(
                            "Failed to clone '{}' from '{}' to '{}'",
                            config_name, config.url, clone_path
                        );
                        io::stderr().write_all(&clone_command.stderr).unwrap();
                    }
                }
            }
        }
        Commands::Start => {
            // Check if the comphost network exists
            let network_check_command = Command::new("docker")
                .args(&["network", "inspect", "comphost"])
                .output()
                .expect("Failed to execute docker network inspect command");

            if !network_check_command.status.success() {
                // Create the comphost network if it does not exist
                let create_network_command = Command::new("docker")
                    .args(&["network", "create", "comphost"])
                    .output()
                    .expect("Failed to execute docker network create command");

                if create_network_command.status.success() {
                    println!("Created comphost network");
                } else {
                    eprintln!("Failed to create comphost network");
                    io::stderr()
                        .write_all(&create_network_command.stderr)
                        .unwrap();
                    return;
                }
            }

            for (config_name, config) in &mut toml_content {
                if config.active {
                    if let Some(ref clone_path) = config.clone_path {
                        let start_command = Command::new("docker")
                            .arg("compose")
                            .arg("up")
                            .arg("--detach")
                            .current_dir(clone_path)
                            .output()
                            .expect("Failed to execute docker compose up command");

                        if start_command.status.success() {
                            println!("Started Docker Compose for '{}'", config_name);

                            // Retrieve container IDs
                            let ps_output = Command::new("docker")
                                .args(&["compose", "ps", "--format", "{{.ID}}"])
                                .current_dir(clone_path)
                                .output()
                                .expect("Failed to execute docker ps command");
                            let container_ids = String::from_utf8_lossy(&ps_output.stdout);

                            // Attach containers to the comphost network
                            for container_id in container_ids.split_whitespace() {
                                let attach_command = Command::new("docker")
                                    .arg("network")
                                    .arg("connect")
                                    .arg("comphost")
                                    .arg(container_id)
                                    .output()
                                    .expect("Failed to execute docker network connect command");

                                if attach_command.status.success() {
                                    println!(
                                        "Attached container '{}' to comphost network for '{}'",
                                        container_id, config_name
                                    );
                                } else {
                                    eprintln!(
                                        "Failed to attach container '{}' to comphost network for '{}'",
                                        container_id, config_name
                                    );
                                    io::stderr().write_all(&attach_command.stderr).unwrap();
                                }
                            }
                        } else {
                            eprintln!("Failed to start Docker Compose for '{}'", config_name);
                            io::stderr().write_all(&start_command.stderr).unwrap();
                        }
                    }
                }
            }
        }
        Commands::Stop => {
            for (config_name, config) in &toml_content {
                if config.active {
                    if let Some(ref clone_path) = config.clone_path {
                        let stop_command = Command::new("docker")
                            .arg("compose")
                            .arg("down")
                            .current_dir(clone_path)
                            .output()
                            .expect("Failed to execute docker compose down command");

                        if stop_command.status.success() {
                            println!("Stopped Docker Compose for '{}'", config_name);
                        } else {
                            eprintln!("Failed to stop Docker Compose for '{}'", config_name);
                            io::stderr().write_all(&stop_command.stderr).unwrap();
                        }
                    }
                }
            }
        }
        Commands::ListNames => {
            for config_name in toml_content.keys() {
                println!("{}", config_name);
            }
        }
    }

    // Serialize the updated HashMap back to TOML
    let toml_string = toml::to_string(&toml_content).expect("Could not serialize to TOML");

    // Write the updated TOML content back to the file
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(config_file_path)
        .expect("Could not open file");
    write!(file, "{}", toml_string).expect("Could not write to file");
}
