use serialport::{SerialPortInfo, SerialPortType};
use std::{fmt::Display, iter::zip};

enum Alignment {
    Left,
    Right,
}

struct Column {
    title: String,
    fields: Vec<String>,
    alignment: Alignment,
}

impl Column {
    pub fn new(title: String) -> Self {
        Self {
            title,
            fields: vec![],
            alignment: Alignment::Right,
        }
    }

    pub fn set_alignment(&mut self, alignment: Alignment) {
        self.alignment = alignment;
    }

    pub fn add_field(&mut self, field: String) {
        self.fields.push(field);
    }

    pub fn max_size(&self) -> usize {
        let fields = self.fields.iter().map(|s| s.len()).collect::<Vec<_>>();
        *vec![self.title.len()]
            .iter()
            .chain(&fields)
            .max()
            .unwrap_or(&0)
    }

    pub fn title(&self) -> String {
        match self.alignment {
            Alignment::Left => self.title.clone(),
            Alignment::Right => format!("{:>width$}", self.title, width = self.max_size()),
        }
    }

    pub fn get_field(&self, index: usize) -> Option<String> {
        let field = self.fields.get(index)?;

        match self.alignment {
            Alignment::Left => Some(field.clone()),
            Alignment::Right => Some(format!("{:>width$}", field, width = self.max_size())),
        }
    }
}

struct Table<const N: usize> {
    columns: [Column; N],
}

impl<const N: usize> Table<N> {
    pub fn new(titles: [String; N]) -> Self {
        let mut obj = Self {
            columns: titles.map(|title| Column::new(title)),
        };

        obj.columns[N - 1].set_alignment(Alignment::Left);

        obj
    }

    pub fn add_row(&mut self, fields: [String; N]) {
        zip(self.columns.as_mut(), fields).for_each(|(column, field)| column.add_field(field));
    }

    pub fn number_of_rows(&self) -> usize {
        self.columns[0].fields.len()
    }
}

impl<const N: usize> Display for Table<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = format!(
            "\x1b[1;30m{}\x1b[0m\r\n",
            self.columns
                .iter()
                .map(|column| column.title())
                .collect::<Vec<_>>()
                .join(" | ")
        );

        for row in 0..self.number_of_rows() {
            output += &format!(
                "{}\r\n",
                self.columns
                    .iter()
                    .map(|column| column.get_field(row).unwrap())
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }

        write!(f, "{}", output)
    }
}

fn squash_serial_number(serial_number: String, max_width: usize) -> String {
    if serial_number.len() > (max_width - 3) {
        format!(
            "...{}",
            &serial_number[serial_number.len() - (max_width - 3)..]
        )
    } else {
        serial_number
    }
}

fn list_serial_ports_verbose(ports: Vec<SerialPortInfo>) {
    let serial_number_title = "Serial Number";
    let mut table = Table::new(
        [
            "Serial Port",
            serial_number_title,
            "PID",
            "VID",
            "Manufacturer",
        ]
        .map(|title| title.to_string()),
    );

    for port in ports {
        let SerialPortType::UsbPort(usb_port_info) = port.port_type else {
            continue;
        };

        table.add_row([
            port.port_name,
            squash_serial_number(
                usb_port_info.serial_number.unwrap_or("???".to_string()),
                serial_number_title.len(),
            ),
            usb_port_info.pid.to_string(),
            usb_port_info.vid.to_string(),
            usb_port_info.manufacturer.unwrap_or("???".to_string()),
        ]);
    }

    print!("{}", table);
}

fn list_serial_ports_non_verbose(ports: Vec<SerialPortInfo>) {
    let max_name_width = ports.iter().map(|p| p.port_name.len()).max().unwrap();

    for port in ports {
        let SerialPortType::UsbPort(usb_port_info) = port.port_type else {
            continue;
        };

        println!(
            "{:>name_width$} - {}",
            port.port_name,
            usb_port_info.manufacturer.unwrap_or("???".to_string()),
            name_width = max_name_width,
        );
    }
}

pub fn list_serial_ports(is_verbose: bool) -> Result<(), String> {
    let Ok(ports) = serialport::available_ports() else {
        return Err("No serial ports found".to_string());
    };

    let ports = ports
        .into_iter()
        .filter(|p| matches!(p.port_type, serialport::SerialPortType::UsbPort(_)))
        .collect::<Vec<_>>();

    if ports.is_empty() {
        println!("No serial ports found");
        return Ok(());
    }

    if is_verbose {
        list_serial_ports_verbose(ports);
    } else {
        list_serial_ports_non_verbose(ports);
    }

    return Ok(());
}
