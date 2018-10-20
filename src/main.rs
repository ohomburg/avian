extern crate env_logger;
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate ws;
#[macro_use]
extern crate serde_derive;

use ws::{listen, Handler, Message, Request, Response, Sender};

mod editor;

use editor::*;

const EDITOR_HTML: &str = include_str!("../public/editor.html");
const EDITOR_JS: &str = include_str!("../public/editor.js");

struct Server<'a> {
    out: Sender,
    editor: &'a Editor,
}

impl<'a> Server<'a> {
    fn handle_edit(&mut self, msg: &Message) -> Result<String, &'static str> {
        let edit: Edit = serde_json::from_str(msg.as_text().or(Err("invalid message"))?)
            .or(Err("invalid json"))?;
        self.editor
            .edit(edit)
            .map(|e| serde_json::to_string(&e).unwrap())
    }
}

impl<'a> Handler for Server<'a> {
    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        self.out
            .send(serde_json::to_string(&self.editor.status()).unwrap())
    }

    fn on_message(&mut self, msg: Message) -> ws::Result<()> {
        match self.handle_edit(&msg) {
            Ok(response) => {
                let json = json!({"success": true});
                self.out.send(json.to_string())?;
                self.out.broadcast(response)
            }
            Err(reason) => {
                let json = json!({"success": false,"reason": reason});
                self.out.send(json.to_string())
            }
        }
    }

    fn on_request(&mut self, req: &Request) -> ws::Result<Response> {
        match req.resource() {
            "/" => Ok(Response::new(200, "OK", Vec::from(EDITOR_HTML))),
            "/editor.js" => Ok(Response::new(200, "OK", Vec::from(EDITOR_JS))),
            "/ws" => Response::from_request(req),
            _ => Ok(Response::new(
                404,
                "Not Found",
                Vec::from("404 - not found"),
            )),
        }
    }
}

fn main() {
    env_logger::init();
    let editor = Editor::new();
    listen("0.0.0.0:8080", |out| Server {
        editor: &editor,
        out,
    }).unwrap();
}
