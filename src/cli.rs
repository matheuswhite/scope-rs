use anyhow::Result;
use clap::{ Subcommand};
use crate::Cli;

#[derive(Subcommand)]
pub enum Commands {
    Serial{
        port: String,
        baudrate: u32,
    },
    Ble{
        name_device: String,
        mtu: u32
    },
    Empty,
}

impl Cli {
    pub fn exec(&self) -> Result<(&String, &u32), bool> {
        match &self.command {
            Commands::Serial{port, baudrate} => {
                Ok((port, baudrate))
            },
            Commands::Empty =>{
                println!("Vazio");
                Err(false)
            },
            _ => {
                Err(false)
            },
        }
    }
}
