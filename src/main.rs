use std::path::PathBuf;

use anyhow::Result;
use winit::event_loop::{ControlFlow, EventLoop};

use rasterwarp::app::App;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();
    let image = std::env::args_os().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(image);
    event_loop.run_app(&mut app)?;
    app.finish()
}
