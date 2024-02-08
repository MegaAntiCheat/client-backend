use std::{future::Future, marker::PhantomData};

use futures::future::BoxFuture;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

pub struct EventLoop<S, M, H>
where
    S: Send,
    M: Send + ConsumingStateUpdater<S> + 'static,
    H: HandlerStruct<S, M, M>,
{
    pub sources: Vec<Box<dyn MessageSource<M> + 'static + Send>>,
    pub handlers: Vec<H>,
    pub queue: Vec<M>,
    pub async_tasks: Vec<JoinHandle<Option<M>>>,

    state: PhantomData<S>,
}

impl<S, M, H> EventLoop<S, M, H>
where
    S: Send,
    M: Send + ConsumingStateUpdater<S> + 'static,
    H: Send + HandlerStruct<S, M, M>,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            handlers: Vec::new(),
            queue: Vec::new(),
            async_tasks: Vec::new(),
            state: PhantomData,
        }
    }

    #[must_use]
    pub fn add_source(mut self, source: Box<dyn MessageSource<M> + Send>) -> Self {
        self.sources.push(source);
        self
    }

    #[must_use]
    pub fn add_handler(mut self, handler: impl Into<H>) -> Self {
        self.handlers.push(handler.into());
        self
    }

    pub fn handle_message(&mut self, message: M, state: &mut S) -> Vec<Action<M>> {
        let mut out = Vec::new();

        for h in &mut self.handlers {
            match h.handle_message(state, &message) {
                Some(Handled(Internal::Single(m))) => out.push(m),
                Some(Handled(Internal::Batch(ms))) => out.extend(ms),
                None => {}
            }
        }

        message.update_state(state);

        out
    }

    pub fn handle_messages(&mut self, messages: Vec<M>, state: &mut S) -> Vec<Action<M>> {
        let mut out = Vec::new();

        for m in messages {
            out.extend(self.handle_message(m, state));
        }
        out
    }

    pub async fn execute_cycle(&mut self, state: &mut S) -> Option<()> {
        let mut messages = Vec::new();

        // Check sources
        messages.append(&mut self.queue);
        for s in &mut self.sources {
            while let Some(m) = s.next_message() {
                messages.push(m);
            }
        }

        // Check async tasks
        let mut finished_tasks = Vec::new();
        for (i, j) in self.async_tasks.iter_mut().enumerate() {
            if j.is_finished() {
                finished_tasks.push(i);
                match j.await {
                    Ok(Some(m)) => messages.push(m),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Task paniced: {e}");
                    }
                }
            }
        }
        for i in finished_tasks.into_iter().rev() {
            self.async_tasks.remove(i);
        }

        // Handle messages
        for a in self.handle_messages(messages, state) {
            match a {
                Action::Message(m) => self.queue.push(m),
                Action::Future(f) => {
                    self.async_tasks.push(tokio::task::spawn(f));
                }
            }
        }

        Some(())
    }
}

impl<S, M, H> Default for EventLoop<S, M, H>
where
    S: Send,
    M: Send + ConsumingStateUpdater<S> + 'static,
    H: Send + HandlerStruct<S, M, M>,
{
    fn default() -> Self { Self::new() }
}

pub trait HandlerStruct<S, IM, OM> {
    fn handle_message(&mut self, state: &S, message: &IM) -> Option<Handled<OM>>;
}

pub struct Handled<M>(Internal<Action<M>>);

enum Internal<T> {
    Single(T),
    Batch(Vec<T>),
}

pub enum Action<M> {
    Message(M),
    Future(BoxFuture<'static, Option<M>>),
}

impl<M> From<M> for Action<M> {
    fn from(value: M) -> Self { Self::Message(value) }
}

impl<M> Handled<M> {
    #[must_use]
    pub const fn none() -> Option<Self> { None }

    pub fn single(m: impl Into<M>) -> Option<Self> {
        Some(Self(Internal::Single(Action::Message(m.into()))))
    }

    pub fn future(future: impl Future<Output = Option<M>> + 'static + Send) -> Option<Self> {
        Some(Self(Internal::Single(Action::Future(Box::pin(future)))))
    }

    pub fn multiple(commands: impl IntoIterator<Item = Option<Self>>) -> Option<Self> {
        let mut batch = Vec::new();

        for maybe_handled in commands {
            match maybe_handled {
                None => {}
                Some(Self(Internal::Single(command))) => batch.push(command),
                Some(Self(Internal::Batch(commands))) => batch.extend(commands),
            }
        }

        Some(Self(Internal::Batch(batch)))
    }
}

#[allow(unused_variables)]
pub trait StateUpdater<S>: Sized {
    fn update_state(self, state: &mut S);
}

#[allow(unused_variables)]
impl<M, S> StateUpdater<S> for &M {
    fn update_state(self, state: &mut S) {}
}

pub trait ConsumingStateUpdater<S>
where
    Self: Sized,
{
    fn update_state(self, _: &mut S) {}
}

pub trait MessageSource<M> {
    fn next_message(&mut self) -> Option<M>;
}

impl<M, I: Into<M>> MessageSource<M> for UnboundedReceiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

impl<M, I: Into<M>> MessageSource<M> for std::sync::mpsc::Receiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

impl<M, I: Into<M>> MessageSource<M> for tokio::sync::mpsc::Receiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

pub fn try_get<T>(message: &impl Is<T>) -> Option<&T> { message.try_get() }

pub trait Is<T>: From<T> {
    fn is(&self) -> bool;
    fn try_get(&self) -> Option<&T>;
}

// Define Handler struct
#[macro_export]
macro_rules! define_handlers {
    ($enum:ident <$state:ty, $message:ty>: $($handler:ident),+) => {
        // Define enum
        pub enum $enum {
            $($handler($handler)),+
        }

        // Impl HandlerStruct<State, Message>
        impl HandlerStruct<$state, $message, $message> for $enum {
            fn handle_message(&mut self, state: &$state, message: &$message) -> Option<Handled<$message>> {
                match self {
                    $($enum::$handler(inner) => inner.handle_message(state, message)),+
                }
            }
        }

        $(
            impl From<$handler> for $enum {
                fn from(val: $handler) -> Self {
                    Self::$handler(val)
                }
            }
        )+
    };
}

// Define Message struct
#[macro_export]
macro_rules! define_messages {
    ($enum:ident <$state:ty>: $($message:ident),+) => {
        // Define enum
        #[derive(Debug)]
        pub enum $enum {
            None,
            $($message($message)),+
        }

        impl Default for $enum {
            fn default() -> $enum {
                $enum::None
            }
        }

        // Impl update_state
        impl event_loop::ConsumingStateUpdater<$state> for $enum {
            fn update_state(self, state: &mut $state) {
                use $enum::*;
                match self {
                    None => {}
                    $($message(i) => i.update_state(state)),+
                }
            }
        }

        // Impl Is
        $(
            impl event_loop::Is<$message> for $enum {
                fn is(&self) -> bool {
                    match self {
                        $enum::$message(_) => true,
                        _ => false,
                    }
                }
                fn try_get(&self) -> Option<&$message> {
                    match self {
                        $enum::$message(a) => Some(a),
                        _ => None,
                    }
                }
            }
        )+

        // Impl Into
        $(
            impl From<$message> for $enum {
                fn from(val: $message) -> $enum {
                    $enum::$message(val)
                }
            }
        )+

        // Impl TryInto
        $(
            impl TryInto<$message> for $enum {
                type Error = ();
                fn try_into(self) -> Result<$message, Self::Error> {
                    match self {
                        $enum::$message(out) => Ok(out),
                        _ => Err(())
                    }
                }
            }
        )+
    };
}
