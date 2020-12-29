mod pe_window;

use std::num::NonZeroU32;

use ahash::AHashMap;
use font_loader::system_fonts;
use fontdue::Font;
use pe_window::PeWindow;
use x11rb::protocol::xproto::{EventMask, ExposeEvent, KeyPressEvent, KEY_PRESS_EVENT};
use xim::{
    x11rb::{HasConnection, X11rbServer},
    InputStyle, Server, ServerHandler,
};

use crate::engine::{DubeolSik, InputEngine, InputResult};

fn load_font() -> Font {
    if let Ok(bytes) = std::fs::read("/usr/share/fonts/TTF/D2Coding.ttc") {
        Font::from_bytes(
            bytes,
            fontdue::FontSettings {
                enable_offset_bounding_box: true,
                scale: 15.0,
                collection_index: 0,
            },
        )
        .unwrap()
    } else {
        let prop = system_fonts::FontPropertyBuilder::new()
            .family("Noto Sans")
            .family("D2Coding")
            .build();
        let (bytes, idx) = system_fonts::get(&prop).expect("Loading fonts");

        Font::from_bytes(
            bytes,
            fontdue::FontSettings {
                enable_offset_bounding_box: true,
                scale: 15.0,
                collection_index: idx as _,
            },
        )
        .unwrap()
    }
}

pub struct KimeData {
    engine: InputEngine<DubeolSik>,
    pe: Option<NonZeroU32>,
}

impl KimeData {
    pub fn new(pe: Option<NonZeroU32>) -> Self {
        Self {
            engine: InputEngine::new(DubeolSik::new()),
            pe,
        }
    }
}

pub struct KimeHandler {
    font: Font,
    preedit_windows: AHashMap<NonZeroU32, PeWindow>,
}

impl KimeHandler {
    pub fn new() -> Self {
        Self {
            font: load_font(),
            preedit_windows: AHashMap::new(),
        }
    }
}

impl KimeHandler {
    pub fn expose<C: HasConnection>(
        &mut self,
        c: C,
        e: ExposeEvent,
    ) -> Result<(), xim::ServerError> {
        if let Some(win) = NonZeroU32::new(e.window) {
            if let Some(pe) = self.preedit_windows.get_mut(&win) {
                pe.expose(c, e)?;
            }
        }

        Ok(())
    }

    fn preedit<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        ic: &mut xim::InputContext<KimeData>,
        ch: char,
    ) -> Result<(), xim::ServerError> {
        if ic.input_style().contains(InputStyle::PREEDITCALLBACKS) {
            log::trace!("Preedit callback {}", ch);
            // on-the-spot send preedit callback
            let mut buf = [0; 4];
            let s = ch.encode_utf8(&mut buf);
            server.preedit_draw(ic, s)?;
        } else if let Some(pe) = ic.user_data.pe.as_mut() {
            log::trace!("Preedit draw {}", ch);
            // off-the-spot draw in server

            self.preedit_windows.get_mut(pe).unwrap().set_preedit(ch);
        }

        Ok(())
    }

    fn commit<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        ic: &mut xim::InputContext<KimeData>,
        ch: char,
    ) -> Result<(), xim::ServerError> {
        let mut buf = [0; 4];
        let s = ch.encode_utf8(&mut buf);
        server.commit(ic, s)?;
        Ok(())
    }
}

impl<C: HasConnection> ServerHandler<X11rbServer<C>> for KimeHandler {
    type InputStyleArray = [InputStyle; 7];
    type InputContextData = KimeData;

    fn new_ic_data(
        &mut self,
        server: &mut X11rbServer<C>,
        input_style: InputStyle,
    ) -> Result<Self::InputContextData, xim::ServerError> {
        if input_style.contains(InputStyle::PREEDITCALLBACKS) {
            // on-the-spot
            Ok(KimeData::new(None))
        } else {
            // other
            let pe = PeWindow::new(&*server)?;
            let win = pe.window();
            self.preedit_windows.insert(win, pe);
            Ok(KimeData::new(Some(win)))
        }
    }

    fn input_styles(&self) -> Self::InputStyleArray {
        [
            // root
            InputStyle::PREEDITNOTHING | InputStyle::PREEDITNOTHING,
            // off-the-spot
            InputStyle::PREEDITPOSITION | InputStyle::STATUSAREA,
            InputStyle::PREEDITPOSITION | InputStyle::STATUSNOTHING,
            InputStyle::PREEDITPOSITION | InputStyle::STATUSNONE,
            // on-the-spot
            InputStyle::PREEDITCALLBACKS | InputStyle::STATUSAREA,
            InputStyle::PREEDITCALLBACKS | InputStyle::STATUSNOTHING,
            InputStyle::PREEDITCALLBACKS | InputStyle::STATUSNONE,
        ]
    }

    fn handle_connect(&mut self, _server: &mut X11rbServer<C>) -> Result<(), xim::ServerError> {
        Ok(())
    }

    fn handle_set_ic_values(
        &mut self,
        _server: &mut X11rbServer<C>,
        _input_context: &mut xim::InputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        Ok(())
    }

    fn handle_create_ic(
        &mut self,
        server: &mut X11rbServer<C>,
        input_context: &mut xim::InputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        log::info!(
            "IC created style: {:?}, spot_location: {:?}",
            input_context.input_style(),
            input_context.preedit_spot()
        );
        server.set_event_mask(
            input_context,
            EventMask::KeyPress | EventMask::KeyRelease,
            0,
            // EventMask::KeyPress | EventMask::KeyRelease,
        )?;

        Ok(())
    }

    fn handle_reset_ic(
        &mut self,
        _server: &mut X11rbServer<C>,
        input_context: &mut xim::InputContext<Self::InputContextData>,
    ) -> Result<String, xim::ServerError> {
        Ok(input_context.user_data.engine.reset())
    }

    fn handle_forward_event(
        &mut self,
        server: &mut X11rbServer<C>,
        input_context: &mut xim::InputContext<Self::InputContextData>,
        xev: &KeyPressEvent,
    ) -> Result<bool, xim::ServerError> {
        if xev.response_type == KEY_PRESS_EVENT {
            let shift = (xev.state & 0x1) != 0;
            let ctrl = (xev.state & 0x4) != 0;

            let ret = input_context
                .user_data
                .engine
                .key_press(xev.detail, shift, ctrl);
            log::trace!("ret: {:?}", ret);

            match ret {
                InputResult::Bypass => Ok(false),
                InputResult::Consume => Ok(true),
                InputResult::CommitBypass(ch) => {
                    self.commit(server, input_context, ch)?;
                    Ok(false)
                }
                InputResult::Commit(ch) => {
                    self.commit(server, input_context, ch)?;
                    Ok(true)
                }
                InputResult::CommitPreedit(commit, preedit) => {
                    self.preedit(server, input_context, preedit)?;
                    self.commit(server, input_context, commit)?;
                    Ok(true)
                }
                InputResult::Preedit(preedit) => {
                    self.preedit(server, input_context, preedit)?;
                    Ok(true)
                }
            }
        } else {
            Ok(false)
        }
    }

    fn handle_destory_ic(
        &mut self,
        server: &mut X11rbServer<C>,
        input_context: xim::InputContext<Self::InputContextData>,
    ) -> Result<(), xim::ServerError> {
        if let Some(pe) = input_context.user_data.pe {
            self.preedit_windows.remove(&pe).unwrap().clean(&*server)?;
        }

        Ok(())
    }

    fn handle_preedit_start(
        &mut self,
        _server: &mut X11rbServer<C>,
        _input_context: &mut xim::InputContext<Self::InputContextData>,
    ) -> Result<(), xim::ServerError> {
        log::info!("preedit started");

        Ok(())
    }

    fn handle_caret(
        &mut self,
        _server: &mut X11rbServer<C>,
        _input_context: &mut xim::InputContext<Self::InputContextData>,
        _position: i32,
    ) -> Result<(), xim::ServerError> {
        Ok(())
    }
}
