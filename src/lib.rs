#![deny(clippy::all)]
mod brush;
mod cmd;
mod font;
mod framebuffer2d;
mod gpu;
mod palette;
mod renderer;
mod resources;
mod screen2d;
mod session;
mod view;

use renderer::Renderer;
use resources::ResourceManager;
use session::*;
use view::FileStatus;

use rgx;
use rgx::core;
use rgx::kit;
use rgx::kit::shape2d;

use rgx::winit;

#[macro_use]
extern crate log;

use env_logger;

use xdg;

use std::collections::VecDeque;
use std::path::Path;
use std::time;

pub struct FrameTimer {
    timings: VecDeque<u128>,
    avg: time::Duration,
    last: time::Instant,
}

impl FrameTimer {
    const WINDOW: usize = 60;

    pub fn new() -> Self {
        Self {
            timings: VecDeque::with_capacity(Self::WINDOW),
            avg: time::Duration::from_secs(0),
            last: time::Instant::now(),
        }
    }

    pub fn run<F>(&mut self, frame: F)
    where
        F: FnOnce(time::Duration, time::Duration),
    {
        let start = time::Instant::now();
        frame(self.avg, start.duration_since(self.last));
        let elapsed = start.elapsed();

        self.last = start;
        self.timings.truncate(Self::WINDOW - 1);
        self.timings.push_front(elapsed.as_micros());

        let avg =
            self.timings.iter().sum::<u128>() / self.timings.len() as u128;
        self.avg = time::Duration::from_micros(avg as u64);
    }
}

pub fn init<P: AsRef<Path>>(paths: &[P]) -> std::io::Result<()> {
    let mut logger = env_logger::Builder::from_default_env();
    logger.init();

    let mut events_loop = winit::EventsLoop::new();
    let win = winit::WindowBuilder::new()
        .with_title("rx")
        .with_resizable(true)
        .build(&events_loop)
        .unwrap();
    let hidpi_factor = win.get_hidpi_factor();
    let win_size = win.get_inner_size().unwrap().to_physical(hidpi_factor);
    let (win_w, win_h) = (win_size.width as u32, win_size.height as u32);

    let resources = ResourceManager::new();
    let base_dirs = xdg::BaseDirectories::with_prefix("rx")?;
    let mut session =
        Session::new(win_w, win_h, hidpi_factor, resources.clone(), base_dirs)
            .init()?;

    let mut present_mode = session.settings.present_mode();
    let mut r = core::Renderer::new(&win);
    let mut renderer = Renderer::new(&mut r, win_w, win_h, resources);

    if let Err(e) = session.edit(paths) {
        session.message(
            format!("Error loading path(s): {}", e),
            MessageType::Error,
        );
    }
    if session.views.is_empty() {
        session.blank(
            FileStatus::NoFile,
            Session::DEFAULT_VIEW_W,
            Session::DEFAULT_VIEW_H,
        );
    }

    renderer.init(&session, &mut r);

    let mut swap_chain = r.swap_chain(
        renderer.width as u32,
        renderer.height as u32,
        present_mode,
    );

    let mut timer = FrameTimer::new();
    let mut canvas = shape2d::Batch::new();
    let mut events = Vec::with_capacity(16);

    while session.is_running {
        timer.run(|avg, delta| {
            canvas.clear();

            events_loop.poll_events(|event| {
                if let rgx::winit::Event::WindowEvent { event, .. } = event {
                    match event {
                        rgx::winit::WindowEvent::Resized(size) => {
                            session.handle_resized(size);

                            let physical = size.to_physical(hidpi_factor);
                            swap_chain = r.swap_chain(
                                physical.width as u32,
                                physical.height as u32,
                                present_mode,
                            );
                            renderer.handle_resized(size, &r);
                        }
                        rgx::winit::WindowEvent::CursorEntered { .. } => {
                            win.hide_cursor(true);
                        }
                        rgx::winit::WindowEvent::CursorLeft { .. } => {
                            win.hide_cursor(false);
                        }
                        other => {
                            events.push(other);
                        }
                    }
                }
            });

            session.frame(&mut events, &mut canvas, delta);
            renderer.frame(&session, &avg, &mut r, &mut swap_chain, &canvas);

            {
                let pm = session.settings.present_mode();
                if pm != present_mode {
                    present_mode = pm;

                    swap_chain = r.swap_chain(
                        session.width as u32,
                        session.height as u32,
                        present_mode,
                    );
                }
            }

            if session.settings.frame_delay > time::Duration::from_secs(0) {
                std::thread::sleep(session.settings.frame_delay);
            }
        });
    }
    Ok(())
}
