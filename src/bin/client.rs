extern crate avian;
#[macro_use]
extern crate clap;
extern crate serde;
extern crate serde_json;
extern crate ws;

use avian::{Edit, EditAction};
use clap::{App, AppSettings, Arg, SubCommand};
use serde_json::Value as Json;

fn main() {
    // rustfmt does not like the way this clap code is formatted. Make it ignore that.
    #[cfg_attr(rustfmt, rustfmt_skip)]
    let matches = {
        App::new("avian-client")
            .version(crate_version!())
            .setting(AppSettings::SubcommandRequired)
            .subcommand(SubCommand::with_name("insert")
                .alias("i")
                .arg(Arg::with_name("position")
                    .help("Byte position at which to insert")
                    .required(true))
                .arg(Arg::with_name("text")
                    .help("Text to insert")
                    .required(true)))
            .subcommand(SubCommand::with_name("delete")
                .alias("d")
                .arg(Arg::with_name("position")
                    .help("Byte position from which to delete")
                    .required(true))
                .arg(Arg::with_name("length")
                    .help("Number of bytes to delete")
                    .required(true)))
            .subcommand(SubCommand::with_name("read")
                .alias("r"))
            .subcommand(SubCommand::with_name("wait")
                .alias("w"))
            .arg(Arg::with_name("host")
                .long("host")
                .short("H")
                .help("Hostname of the server")
                .takes_value(true)
                .default_value("localhost"))
            .arg(Arg::with_name("port")
                .long("port")
                .short("p")
                .help("Port of the server")
                .takes_value(true)
                .default_value("8080"))
            .arg(Arg::with_name("secure")
                .long("secure")
                .short("s")
                .help("Set to use encryption"))
            .arg(Arg::with_name("revisions")
                .long("rev")
                .short("r")
                .help("Show revision numbers received"))
            .get_matches()
    };

    let protocol = if matches.is_present("secure") {
        "wss"
    } else {
        "ws"
    };
    let show_rev = matches.is_present("revisions");
    let host = matches.value_of("host").unwrap();
    let port = matches.value_of("port").unwrap();
    let url = format!("{}://{}:{}/ws", protocol, host, port);

    match matches.subcommand_name().unwrap() {
        "read" => {
            ws::connect(url, |out| {
                move |msg: ws::Message| {
                    let (rev, buffer) = serde_json::from_str::<(u32, String)>(msg.as_text()?)
                        .expect("TODO: graceful shutdown.");
                    if show_rev {
                        println!("Rev {}", rev);
                    }
                    println!("{}", buffer);
                    out.close(ws::CloseCode::Normal)
                }
            }).unwrap();
        }
        "insert" => {
            let sub_matches = matches.subcommand_matches("insert").unwrap();
            let pos = sub_matches
                .value_of("position")
                .unwrap()
                .parse::<usize>()
                .expect("position must be a number");
            let text = sub_matches.value_of("text").unwrap();
            ws::connect(url, move |out| ActionClient {
                show_rev,
                out,
                pos,
                action: EditAction::Insert(text.to_string()),
                init_received: false,
            }).unwrap();
        }
        "delete" => {
            let sub_matches = matches.subcommand_matches("delete").unwrap();
            let pos = sub_matches
                .value_of("position")
                .unwrap()
                .parse::<usize>()
                .expect("position must be a number");
            let len = sub_matches
                .value_of("length")
                .unwrap()
                .parse::<usize>()
                .expect("length must be a number");
            ws::connect(url, |out| ActionClient {
                show_rev,
                out,
                pos,
                action: EditAction::Delete(len),
                init_received: false,
            }).unwrap();
        }
        "wait" => {
            ws::connect(url, |_| WaitClient {
                show_rev,
                init_received: false,
            }).unwrap();
        }
        _ => panic!("Unknown subcommand not handled by clap."),
    }
}

struct ActionClient {
    show_rev: bool,
    out: ws::Sender,
    pos: usize,
    action: EditAction,
    init_received: bool,
}

impl ws::Handler for ActionClient {
    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        if !self.init_received {
            let (rev, _) = serde_json::from_str::<(u32, String)>(msg.as_text()?)
                .expect("TODO: graceful shutdown.");
            if self.show_rev {
                println!("Rev {}", rev);
            }
            self.init_received = true;
            let edit = Edit {
                pos: self.pos,
                rev,
                action: self.action.clone(),
            };
            self.out.send(serde_json::to_string(&edit).unwrap())
        } else {
            // wait to receive success
            let json =
                serde_json::from_str::<Json>(msg.as_text()?).expect("TODO: graceful shutdown.");
            if let Json::Object(map) = json {
                if map.contains_key("success") {
                    if Json::Bool(true) != map["success"] {
                        eprintln!("Failed action. Reason: {}", map["reason"]);
                    }
                    self.out.close(ws::CloseCode::Normal)?;
                }
            }
            Ok(())
        }
    }
}

struct WaitClient {
    show_rev: bool,
    init_received: bool,
}

impl ws::Handler for WaitClient {
    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        if !self.init_received {
            let (rev, buffer) = serde_json::from_str::<(u32, String)>(msg.as_text()?)
                .expect("TODO: graceful shutdown.");
            if self.show_rev {
                println!("Rev {}", rev);
            }
            println!("Text: {} bytes.\n{}", buffer.len(), buffer);
            self.init_received = true;
        } else {
            // wait to receive success
            let json =
                serde_json::from_str::<Json>(msg.as_text()?).expect("TODO: graceful shutdown.");
            let map = json.as_object().unwrap();
            let pos = map["pos"].as_u64().unwrap() as usize;
            let action: EditAction = serde_json::from_value(map["action"].clone()).unwrap();
            if self.show_rev {
                print!("Rev {}: ", map["rev"].as_u64().unwrap());
            }
            match action {
                EditAction::Insert(txt) => println!("insert({}, {:?})", pos, txt),
                EditAction::Delete(len) => println!("delete({}, {})", pos, len),
            }
        }
        Ok(())
    }
}
