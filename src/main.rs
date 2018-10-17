extern crate ws;

extern crate serde;
extern crate serde_json;
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

impl<'a> Handler for Server<'a> {
    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        self.out
            .send(serde_json::to_string(&self.editor.status()).unwrap())
    }

    fn on_message(&mut self, msg: Message) -> ws::Result<()> {
        let edit: Edit = serde_json::from_str(msg.as_text()?)
            .map_err(|err| ws::Error::new(ws::ErrorKind::Custom(Box::new(err)), "invalid json"))?;
        self.editor.edit(edit);
        Ok(())
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
    let editor = Editor::new();
    listen("0.0.0.0:8080", |out| Server {
        editor: &editor,
        out,
    }).unwrap();
}
