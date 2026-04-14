#[path = "render/frame.rs"]
mod frame;
#[path = "render/queue.rs"]
mod queue;
#[path = "render/task.rs"]
mod task;
#[path = "render/terminal.rs"]
mod terminal;

pub(super) use frame::InitProgressRenderer;
pub(super) use terminal::fit_init_plain_line;

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;
