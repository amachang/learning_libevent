#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::{
    mem::{
        zeroed,
        size_of,
    },
    process::exit,
    ptr::{
        null,
        null_mut,
        NonNull,
    },
    ffi::*,
};

#[derive(Debug)]
struct EventError(String);

#[derive(Debug)]
struct EventBase {
    base: NonNull<event_base>,
}

impl EventBase {
    fn try_new() -> Result<EventBase, EventError> {
        let base: Option<NonNull<event_base>> = NonNull::new(unsafe { event_base_new() });
        match base {
            None => Err(EventError("Could not initialize libevent!".into())),
            Some(base) => Ok(EventBase { base }),
        }
    }

    fn run(&self) {
        unsafe { event_base_dispatch(self.base.as_ptr()) };
    }

    fn exit(&self, sec: f64) {
        let tv_sec = sec.floor() as i64;
        let tv_usec = ((sec - sec.floor()) * 1_000_000f64) as i32;
        let delay: timeval = timeval { tv_sec, tv_usec };
        unsafe { event_base_loopexit(self.base.as_ptr(), &delay) };
    }

    unsafe fn as_ptr(&self) -> *mut event_base {
        self.base.as_ptr()
    }
}

impl Drop for EventBase {
    fn drop(&mut self) {
        unsafe { event_base_free(self.base.as_ptr()) };
    }
}

#[derive(Debug)]
struct ConnectionListener {
    listener: NonNull<evconnlistener>,
}

impl ConnectionListener {
    fn try_new(base: &EventBase, port: u16, listener_cb: impl Fn(i32)) -> Result<ConnectionListener, EventError> {
        let mut sin: sockaddr_in = unsafe { zeroed() };
        sin.sin_family = AF_INET as u8;
        sin.sin_port = Self::htons(port);

        let listener_cb: Box<Box<dyn Fn(i32)>> = Box::new(Box::new(listener_cb));

        extern "C" fn c_listener_cb(_listener: *mut evconnlistener, fd: i32, _sa: *mut sockaddr, _socklen: i32, listener_cb: *mut c_void) {
            let listener_cb: &Box<dyn Fn(i32)> = unsafe { &*(listener_cb as *mut _) };
            listener_cb(fd);
        }

        let listener: Option<NonNull<evconnlistener>> = NonNull::new(unsafe {
            evconnlistener_new_bind(
                base.as_ptr(),
                Some(c_listener_cb),
                Box::into_raw(listener_cb) as *mut _,
                LEV_OPT_REUSEABLE | LEV_OPT_CLOSE_ON_FREE,
                -1,
                &sin as *const sockaddr_in as *const sockaddr,
                size_of::<sockaddr_in>() as i32,
            )
        });
        match listener {
            None => Err(EventError("Could not initialize connection Listener!".into())),
            Some(listener) => Ok(ConnectionListener { listener }),
        }
    }

    fn htons(u: u16) -> u16 {
        u.to_be()
    }
}

impl Drop for ConnectionListener {
    fn drop(&mut self) {
        unsafe { evconnlistener_free(self.listener.as_ptr()) };
    }
}

#[derive(Debug)]
struct SignalListener {
    event: NonNull<event>,
}

impl SignalListener {
    fn try_new(base: &EventBase, sig: u32, listener_cb: impl Fn(i16)) -> Result<SignalListener, EventError> {
        let listener_cb: Box<Box<dyn Fn(i16)>> = Box::new(Box::new(listener_cb));

        extern "C" fn c_listener_cb(_sig: i32, events: i16, listener_cb: *mut c_void) {
            let listener_cb: &Box<dyn Fn(i16)> = unsafe { &*(listener_cb as *mut _) };
            listener_cb(events);
        }

        let event: Option<NonNull<event>> = NonNull::new(unsafe {
            event_new(
                base.as_ptr(),
                sig as i32,
                (EV_SIGNAL | EV_PERSIST) as i16,
                Some(c_listener_cb),
                Box::into_raw(listener_cb) as *mut _
            )
        });

        let Some(event) = event else {
            return Err(EventError("Could not create a signal event!".into()));
        };

        let add_result = unsafe { event_add(event.as_ptr(), null()) };

        if add_result < 0 {
            return Err(EventError("Could not add a signal event!".into()));
        };

        Ok(SignalListener { event })
    }
}

impl Drop for SignalListener {
    fn drop(&mut self) {
        unsafe { event_free(self.event.as_ptr()) };
    }
}

const PORT: u16 = 9995;

fn main() {
    match try_main() {
        Err(err) => {
            eprintln!("Error: {}", err.0);
            exit(1);
        },
        _ => (),
    }
}

fn try_main() -> Result<(), EventError> {
    let base = EventBase::try_new()?;
    let _connection_listener = ConnectionListener::try_new(&base, PORT, |fd: i32| {
        let bev: Option<NonNull<bufferevent>> = NonNull::new(unsafe { bufferevent_socket_new(base.as_ptr(), fd, bufferevent_options_BEV_OPT_CLOSE_ON_FREE as i32) });
        let Some(bev) = bev else {
            eprintln!("Error constructing bufferevent!");
            unsafe { event_base_loopbreak(base.as_ptr()) };
            return;
        };

        extern "C" fn c_write_cb(bev: *mut bufferevent, _user_data: *mut c_void)
        {
            let bev: NonNull<bufferevent> = NonNull::new(bev).expect("buffer event pointer shoudn't be null");
            let output: NonNull<evbuffer> = NonNull::new(unsafe { bufferevent_get_output(bev.as_ptr()) }).expect("event buffer pointer shoudn't be null");
            let remaining_outputs = unsafe { evbuffer_get_length(output.as_ptr()) };
            if remaining_outputs == 0 {
                println!("Answered");
            }
        }

        extern "C" fn c_read_cb(bev: *mut bufferevent, _user_data: *mut c_void) {
            let bev: NonNull<bufferevent> = NonNull::new(bev).expect("buffer event pointer shoudn't be null");
            let evbuf_in: NonNull<evbuffer> = NonNull::new(unsafe { bufferevent_get_input(bev.as_ptr()) }).expect("event buffer pointer shoudn't be null");
            let evbuf_out: NonNull<evbuffer> = NonNull::new(unsafe { bufferevent_get_output(bev.as_ptr()) }).expect("event buffer pointer shoudn't be null");
            loop {
                let inputs = unsafe { evbuffer_get_length(evbuf_in.as_ptr()) };
                let writes = unsafe { evbuffer_remove_buffer(evbuf_in.as_ptr(), evbuf_out.as_ptr(), inputs) } as usize;
                if inputs <= writes {
                    break;
                }
            };
            println!("Received");
        }

        extern "C" fn c_event_cb(bev: *mut bufferevent, events: i16, _user_data: *mut c_void) {
            let bev: NonNull<bufferevent> = NonNull::new(bev).expect("buffer event pointer shoudn't be null");
            if (events & BEV_EVENT_READING as i16) != 0 {
                eprintln!("Error encountered while reading.");
            }

            if (events & BEV_EVENT_WRITING as i16) != 0 {
                eprintln!("Error encountered while writing.");
            }

            if (events & BEV_EVENT_EOF as i16) != 0 {
                eprintln!("Eof reached.");
            }

            if (events & BEV_EVENT_ERROR as i16) != 0 {
                eprintln!("Unrecoverable error encountered.");
            }

            if (events & BEV_EVENT_TIMEOUT as i16) != 0 {
                eprintln!("User-specified timeout reached.");
            }

            if (events & BEV_EVENT_CONNECTED as i16) != 0 {
                eprintln!("Connect operation finished.");
            }

            unsafe { bufferevent_free(bev.as_ptr()) };
        }

        unsafe { bufferevent_setcb(bev.as_ptr(), Some(c_read_cb), Some(c_write_cb), Some(c_event_cb), null_mut()) };
        unsafe { bufferevent_enable(bev.as_ptr(), (EV_WRITE | EV_READ) as i16) };
    })?;

    let _signal_listener = SignalListener::try_new(&base, SIGINT, |_events: i16| {
        println!("Caught an interrupt signal; exiting cleanly in two seconds.");
        base.exit(2.0);
    });

    println!("Start listening the port: {}", PORT);
    base.run();

    println!("done");
    Ok(())
}

