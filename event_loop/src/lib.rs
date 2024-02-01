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

    /// # Panics
    /// If any of the async tasks executed as part of message resolution
    /// paniced.
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
                if let Some(m) = j.await.expect("Task paniced") {
                    messages.push(m);
                }
            }
        }
        for i in finished_tasks.into_iter().rev() {
            self.async_tasks.remove(i);
        }

        if messages.is_empty() {
            return None;
        }

        // Run handlers
        for h in &mut self.handlers {
            for m in &messages {
                let mut actions = Vec::new();
                match h.handle_message(state, m) {
                    None => {}
                    Some(Handled(Internal::Single(a))) => actions.push(a),
                    Some(Handled(Internal::Batch(a))) => actions = a,
                }

                for a in actions {
                    match a {
                        Action::Message(m) => self.queue.push(m),
                        Action::Future(f) => {
                            self.async_tasks.push(tokio::task::spawn(f));
                        }
                    }
                }
            }
        }

        // Update state
        for m in messages {
            m.update_state(state);
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

enum Action<M> {
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
            $($message($message)),+
        }

        // Impl update_state
        impl event_loop::ConsumingStateUpdater<$state> for $enum {
            fn update_state(self, state: &mut $state) {
                use $enum::*;
                match self {
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
