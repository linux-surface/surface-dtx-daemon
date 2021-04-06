
// for logging with dynamic level
macro_rules! event {
    (target: $target:expr, $lvl:expr, $($k:ident).+ = $($fields:tt)* ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(target: $target, ::tracing::Level::ERROR, $($k).+ = $($fields)*),
            ::tracing::Level::WARN  => ::tracing::event!(target: $target, ::tracing::Level::WARN,  $($k).+ = $($fields)*),
            ::tracing::Level::INFO  => ::tracing::event!(target: $target, ::tracing::Level::INFO,  $($k).+ = $($fields)*),
            ::tracing::Level::DEBUG => ::tracing::event!(target: $target, ::tracing::Level::DEBUG, $($k).+ = $($fields)*),
            ::tracing::Level::TRACE => ::tracing::event!(target: $target, ::tracing::Level::TRACE, $($k).+ = $($fields)*),
        }
    };

    (target: $target:expr, $lvl:expr, $($arg:tt)+ ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(target: $target, ::tracing::Level::ERROR, $($arg)+),
            ::tracing::Level::WARN  => ::tracing::event!(target: $target, ::tracing::Level::WARN,  $($arg)+),
            ::tracing::Level::INFO  => ::tracing::event!(target: $target, ::tracing::Level::INFO,  $($arg)+),
            ::tracing::Level::DEBUG => ::tracing::event!(target: $target, ::tracing::Level::DEBUG, $($arg)+),
            ::tracing::Level::TRACE => ::tracing::event!(target: $target, ::tracing::Level::TRACE, $($arg)+),
        }
    };

    ($lvl:expr, $($k:ident).+ = $($field:tt)*) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(::tracing::Level::ERROR, $($k).+ = $($field)*),
            ::tracing::Level::WARN  => ::tracing::event!(::tracing::Level::WARN,  $($k).+ = $($field)*),
            ::tracing::Level::INFO  => ::tracing::event!(::tracing::Level::INFO,  $($k).+ = $($field)*),
            ::tracing::Level::DEBUG => ::tracing::event!(::tracing::Level::DEBUG, $($k).+ = $($field)*),
            ::tracing::Level::TRACE => ::tracing::event!(::tracing::Level::TRACE, $($k).+ = $($field)*),
        }
    };

    ( $lvl:expr, $($arg:tt)+ ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(::tracing::Level::ERROR, $($arg)+),
            ::tracing::Level::WARN  => ::tracing::event!(::tracing::Level::WARN,  $($arg)+),
            ::tracing::Level::INFO  => ::tracing::event!(::tracing::Level::INFO,  $($arg)+),
            ::tracing::Level::DEBUG => ::tracing::event!(::tracing::Level::DEBUG, $($arg)+),
            ::tracing::Level::TRACE => ::tracing::event!(::tracing::Level::TRACE, $($arg)+),
        }
    };
}
