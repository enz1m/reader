use super::Msg;
use iced::Task;
use iced_test::futures::futures::{StreamExt, executor::block_on};

#[derive(Debug, Default)]
pub(super) struct ObservedTask {
    pub(super) messages: Vec<Msg>,
    pub(super) closes: Vec<iced::window::Id>,
    pub(super) other_actions: usize,
    pub(super) widget_operations: usize,
    pub(super) image_allocations: usize,
}

impl ObservedTask {
    pub(super) fn assert_quiet(&self) {
        assert!(self.messages.is_empty(), "messages: {:?}", self.messages);
        assert!(
            self.closes.is_empty(),
            "premature closes: {:?}",
            self.closes
        );
        assert_eq!(self.other_actions, 0, "unexpected runtime actions");
        assert_eq!(self.image_allocations, 0, "unexpected image uploads");
    }
}

pub(super) fn observe(task: Task<Msg>) -> ObservedTask {
    block_on(observe_async(task))
}

async fn observe_async(task: Task<Msg>) -> ObservedTask {
    let mut observed = ObservedTask::default();
    let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
        return observed;
    };
    // A request can own the sender its continuation awaits; drop it before polling again.
    while let Some(action) = stream.next().await {
        match action {
            iced_test::runtime::Action::Output(message) => observed.messages.push(message),
            iced_test::runtime::Action::Window(iced_test::runtime::window::Action::Close(id)) => {
                observed.closes.push(id);
            }
            // Answered like a renderer that cannot hold the image, so a task
            // waiting on an allocation still delivers its message.
            iced_test::runtime::Action::Image(iced_test::runtime::image::Action::Allocate(
                _,
                sender,
            )) => {
                let _ = sender.send(Err(iced::widget::image::Error::Unsupported));
                observed.image_allocations += 1;
            }
            iced_test::runtime::Action::Widget(_) | iced_test::runtime::Action::WidgetIn { .. } => {
                observed.widget_operations += 1;
                observed.other_actions += 1;
            }
            _ => observed.other_actions += 1,
        }
    }
    observed
}
