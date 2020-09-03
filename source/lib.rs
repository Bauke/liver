//! # Liver
//!
//! > Quick and dirty live reloading server for web development.
//!
//! This library provides one function [`watch`] that does the following:
//!
//! * Creates a WebSocket server (with [`ws`]).
//! * Creates a file watcher (with [`hotwatch`]) that detects file changes and
//! sends a WebSocket message whenever one happens.
//! * Creates a small HTTP server (with [`rocket`]) that returns static files
//! from the path you passed through [`watch`]. And it injects some JavaScript
//! in any HTML files to reload the page when a WebSocket message is received.
//!
//! **Note**: this was developed in a very quick and dirty manner, please don't
//! use this in a production setting. It doesn't have proper error handling
//! (`unwrap`s everywhere) and isn't thoroughly tested. If you're interested in
//! improving the code and stability, I very much welcome it. I think I've
//! gotten about as far as I can with my limited Rust capabilities.
//!
//! To change the ports used for Rocket or the WebSocket server, you can set
//! their corresponding environment variables: `ROCKET_PORT` (8000 by default)
//! and `WS_PORT` (8001 by default).

#![feature(proc_macro_hygiene, decl_macro)]

use std::{env, ffi::OsStr, fs::read, io::Cursor, path::PathBuf, thread};

#[macro_use]
extern crate rocket;

use anyhow::Result;
use hotwatch::{notify::DebouncedEvent, Hotwatch};
use rocket::{
  http::{ContentType, Status},
  response, Rocket, State,
};

/// The reload JavaScript that gets injected into HTML files so we can do
/// `location.reload()` when the server detects changes.
pub(crate) const RELOAD_SCRIPT: &str = r#"<script>
const socket = new WebSocket("ws://127.0.0.1:${WS_PORT}");

socket.addEventListener('error', (event) => {
  console.error(event);
});

socket.addEventListener('open', (event) => {
  console.debug("Liver: reloading enabled!");
});

socket.addEventListener('message', (event) => {
  if (event.data === 'Reload') {
    console.debug('Liver: reloading...');
    location.reload();
  }
});
</script>"#;

/// The default websocket, I picked 8001 as the default Rocket port is 8000.
///
/// Both the Rocket and WS ports can be overridden with `ROCKET_PORT` and
/// `WS_PORT` environment variables.
pub(crate) const WS_PORT_DEFAULT: &str = "8001";

/// The watch function, see the [top-level module documentation](crate) for info.
pub fn watch(path: &str) -> Result<()> {
  let new_path = path.to_string();

  // Use a separate thread to run the websocket server and file watcher in.
  thread::spawn(move || {
    let mut watcher = Hotwatch::new().unwrap();

    // Start the websocket server.
    ws::listen(ws_url(), move |out| {
      // Then whenever we have a connection, start watching the source.
      // I'm *pretty sure* this is fine, as far as I can tell Hotwatch just
      // overrides any old watchers on the same path.
      // I could be very wrong though!
      watcher
        .watch(&new_path, move |event| {
          // Then, whenever Hotwatch notices an event, send the reload message.
          if let DebouncedEvent::Write(_) = event {
            out.send("Reload").unwrap();
          }
        })
        .unwrap();
      |_| Ok(())
    })
    .unwrap();
  });

  // Start Rocket, this will block the main thread.
  Rocket::ignite()
    .manage(path.to_string())
    .mount("/", routes![index, static_files])
    .launch();

  Ok(())
}

/// Small convenience function to return the websocket URL.
pub(crate) fn ws_url() -> String {
  format!(
    "127.0.0.1:{}",
    env::var("WS_PORT").unwrap_or_else(|_| WS_PORT_DEFAULT.into())
  )
}

/// The regular index needs to be handled specifically, it just relays to
/// `static_files` though. *shrug*
#[get("/")]
pub(crate) fn index<'r>(source: State<String>) -> response::Result<'r> {
  static_files(None, source)
}

#[get("/<path..>")]
pub(crate) fn static_files<'r>(
  path: Option<PathBuf>,
  source: State<String>,
) -> response::Result<'r> {
  // Grab the reload JavaScript and set the WS_PORT in it.
  let mut reload_script = RELOAD_SCRIPT
    .replace(
      "${WS_PORT}",
      &env::var("WS_PORT").unwrap_or_else(|_| WS_PORT_DEFAULT.to_string()),
    )
    .as_bytes()
    .to_vec();

  if path.is_none() {
    // If `path` is None that means it was called from `index`, so we just return
    // the `index.html` at the `source` root or a 404 if it doesn't exist.
    let path = PathBuf::from(source.inner()).join("index.html");

    if let Ok(mut file) = read(&path) {
      // Insert the reload JavaScript since we're going to be returning HTML.
      file.append(&mut reload_script);

      return response::Response::build()
        .header(ContentType::HTML)
        .sized_body(Cursor::new(file))
        .ok();
    } else {
      return Err(Status::NotFound);
    }
  }

  // Join our `source` path with the URL `path` so we get the correct
  // relative URL.
  let mut path = PathBuf::from(source.inner()).join(path.unwrap());

  // If it's pointing to a directory then join `index.html`.
  if path.is_dir() {
    path = path.join("index.html");
  }

  if let Ok(mut file) = read(&path) {
    // Get the extension of the file, if any.
    let file_extension = path.extension().and_then(OsStr::to_str).unwrap_or("");

    // Get the content type and use plaintext if we can't find it.
    let content_type =
      ContentType::from_extension(file_extension).unwrap_or(ContentType::Plain);

    if content_type == ContentType::HTML {
      // If we're about to return HTML, insert the reload JavaScript.
      file.append(&mut reload_script);
    }

    response::Response::build()
      .header(content_type)
      .sized_body(Cursor::new(file))
      .ok()
  } else {
    Err(Status::NotFound)
  }
}
