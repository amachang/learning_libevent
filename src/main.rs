#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::{
    collections::*,
    marker::*,
    rc::*,
    mem::*,
    ptr::*,
    ffi::*,
    cell::*,
};
// use debug_cell::RefCell;

#[derive(Clone)]
struct EventError(String);

struct EventLoopDataHolder {
    base: NonNull<event_base>,
    connection_ctx_ptrs: Vec<NonNull<CallbackContext<Box<dyn Fn(i32)>, ()>>>,
    connection_listeners: Vec<NonNull<evconnlistener>>,
    signal_ctx_ptrs: Vec<NonNull<CallbackContext<Box<dyn Fn(u32, i16)>, u32>>>,
    signal_events: Vec<NonNull<event>>,
    socket_map: HashMap<i32, Rc<Socket>>,
    socket_errs: Vec<EventError>,
    break_reason_err: Option<EventError>,
}

impl EventLoopDataHolder {
    fn new(base: NonNull<event_base>) -> Self {
        Self {
            base,
            connection_ctx_ptrs: vec![],
            connection_listeners: vec![],
            signal_ctx_ptrs: vec![],
            signal_events: vec![],
            socket_map: HashMap::new(),
            socket_errs: vec![],
            break_reason_err: None,
        }
    }

    fn base_ptr(&self) -> *mut event_base {
        self.base.as_ptr()
    }
}

impl Drop for EventLoopDataHolder {
    fn drop(&mut self) {
        // drop all sockets
        self.socket_map = HashMap::new();

        unsafe { event_base_free(self.base.as_ptr()) }

        for ctx_ptr in &self.connection_ctx_ptrs {
            // when dropping box, free()
            unsafe { Box::from_raw(ctx_ptr.as_ptr()) };
        }
        for listener in &self.connection_listeners {
            unsafe { evconnlistener_free(listener.as_ptr()) };
        }

        for ctx_ptr in &self.signal_ctx_ptrs {
            // when dropping box, free()
            unsafe { Box::from_raw(ctx_ptr.as_ptr()) };
        }
        for event in &self.signal_events {
            unsafe { event_free(event.as_ptr()) };
        }
        println!("free all pointers");
    }
}

struct EventLoop {
    data: RefCell<EventLoopDataHolder>,
}

impl EventLoop {
    fn try_new() -> Result<Rc<Self>, EventError> {
        let Some(base) = NonNull::new(unsafe { event_base_new() }) else {
            return Err(EventError("Couldn't initialize event loop".into()));
        };
        let data = RefCell::new(EventLoopDataHolder::new(base));
        Ok(Rc::new(Self { data }))
    }

    fn run(&self) -> Result<(), EventError> {
        let base_ptr = self.data.borrow().base_ptr();
        unsafe { event_base_dispatch(base_ptr) };
        Ok(())
    }

    fn exit(&self, sec: f64) -> Result<(), EventError> {
        let tv_sec = sec.floor() as i64;
        let tv_usec = ((sec - sec.floor()) * 1_000_000f64) as i32;
        let delay: timeval = timeval { tv_sec, tv_usec };
        let base_ptr = self.data.borrow().base_ptr();
        unsafe { event_base_loopexit(base_ptr, &delay) };
        Ok(())
    }

    fn break_with_err(&self, err: EventError) {
        let base_ptr = self.data.borrow().base_ptr();
        unsafe { event_base_loopbreak(base_ptr) };
        self.data.borrow_mut().break_reason_err = Some(err);
    }

    fn bind_inet_port(self: &Rc<Self>, port: u16, cb: impl Fn(i32) -> Result<(), EventError> + 'static) -> Result<(), EventError> {
        let mut sin: sockaddr_in = unsafe { zeroed() };
        sin.sin_family = AF_INET as u8;
        sin.sin_port = port.to_be();
        let sin = sin;

        let self_weak_ref = Rc::downgrade(self);
        let func: Box<dyn Fn(i32)> = Box::new(move |fd| {
            let slf = self_weak_ref.upgrade().expect("Broken prerequisite");
            if let Err(err) = cb(fd) {
                slf.break_with_err(err);
            }
        });
        let ctx = Box::new(CallbackContext {
            func: func,
            arg: (),
        });
        // move into pointer
        let ctx_ptr: *mut CallbackContext<Box<dyn Fn(i32)>, ()> = Box::into_raw(ctx);

        // context free by pointer holder
        self.data.borrow_mut().connection_ctx_ptrs.push(unsafe { NonNull::new_unchecked(ctx_ptr) });

        let base_ptr = self.data.borrow().base_ptr();
        let listener: NonNull<evconnlistener> = NonNull::new(unsafe {
            evconnlistener_new_bind(
                base_ptr,
                Some(c_bind_cb),
                ctx_ptr as *mut _,
                LEV_OPT_REUSEABLE | LEV_OPT_CLOSE_ON_FREE,
                -1,
                &sin as *const _ as *const _,
                size_of::<sockaddr_in>() as i32,
            )
        }).expect("Couldn't initialize eveconnlistener");

        self.data.borrow_mut().connection_listeners.push(listener);
        Ok(())
    }

    fn handle_signal(self: &Rc<Self>, sig: u32, cb: impl Fn(u32, i16) -> Result<(), EventError> + 'static) -> Result<(), EventError> {
        let self_weak_ref = Rc::downgrade(self);
        let func: Box<dyn Fn(u32, i16)> = Box::new(move |sig, events| {
            let slf = self_weak_ref.upgrade().expect("Broken prerequisite");
            if let Err(err) = cb(sig, events) {
                slf.break_with_err(err);
            }
        });
        let ctx = Box::new(CallbackContext {
            func: func,
            arg: sig,
        });
        // move into pointer
        let ctx_ptr: *mut CallbackContext<Box<dyn Fn(u32, i16)>, u32> = Box::into_raw(ctx);

        // context free by pointer holder
        self.data.borrow_mut().signal_ctx_ptrs.push(unsafe { NonNull::new_unchecked(ctx_ptr) });

        let base_ptr = self.data.borrow().base_ptr();
        let event: Option<NonNull<event>> = NonNull::new(unsafe {
            event_new(
                base_ptr,
                sig as i32,
                (EV_SIGNAL | EV_PERSIST) as i16,
                Some(c_signal_cb),
                ctx_ptr as *mut _
            )
        });

        let Some(event) = event else {
            return Err(EventError("Could not create a signal event!".into()));
        };

        let add_result = unsafe { event_add(event.as_ptr(), null()) };

        if add_result < 0 {
            unsafe { event_free(event.as_ptr()) };
            return Err(EventError("Could not add a signal event!".into()));
        };

        self.data.borrow_mut().signal_events.push(event);
        Ok(())
    }

    fn try_new_socket(self: &Rc<Self>, fd: i32) -> Result<Rc<Socket>, EventError> {
        let bufferevent: Option<NonNull<bufferevent>> = NonNull::new(unsafe {
            let base_ptr = self.data.borrow().base_ptr();
            bufferevent_socket_new(
                base_ptr,
                fd,
                bufferevent_options_BEV_OPT_CLOSE_ON_FREE as i32,
            )
        });
        let Some(bufferevent) = bufferevent else {
            return Err(EventError("Couldn't initialize socket".into()));
        };

        let self_weak_ref = Rc::downgrade(self);
        let socket = Socket::new(fd, bufferevent, move |socket, result| {
            let slf = self_weak_ref.upgrade().expect("Broken prerequisite");
            let fd = socket.data.borrow().fd;
            slf.data.borrow_mut().socket_map.remove(&fd);
            if let Err(err) = result {
                eprintln!("Socket closed by error: {}", err.0);
                slf.data.borrow_mut().socket_errs.push(err);
            };
        });

        self.data.borrow_mut().socket_map.insert(fd, socket.clone());
        Ok(socket)
    }
}

enum SocketEventKind {
    Read,
    Write,
    Event(i16),
}

struct SocketDataHolder {
    fd: i32,
    bufferevent: NonNull<bufferevent>,
    cb_ctx_ptr: Option<NonNull<CallbackContext<Box<dyn Fn(SocketEventKind)>, ()>>>,
    read_cb: Option<Rc<dyn Fn(Vec<u8>) -> Result<(), EventError>>>,
    close_cb: Option<Box<dyn FnOnce(&Socket, Result<(), EventError>)>>,
}

impl SocketDataHolder {
    fn new(fd: i32, bufferevent: NonNull<bufferevent>) -> Self {
        Self {
            fd,
            bufferevent,
            cb_ctx_ptr: None,
            read_cb: None,
            close_cb: None,
        }
    }
}

impl Drop for SocketDataHolder {
    fn drop(&mut self) {
        unsafe{ bufferevent_free(self.bufferevent.as_ptr()) }

        if let Some(cb_ctx_ptr) = self.cb_ctx_ptr {
            // free()
            unsafe { Box::from_raw(cb_ctx_ptr.as_ptr()) };
        }
        println!("free all socket data");
    }
}

struct Socket {
    data: RefCell<SocketDataHolder>,
}

impl Socket {
    fn new(fd: i32, bufferevent: NonNull<bufferevent>, close_cb: impl FnOnce(&Socket, Result<(), EventError>) + 'static) -> Rc<Self> {
        let data = SocketDataHolder::new(fd, bufferevent);
        let data = RefCell::new(data);
        let socket = Rc::new(Self { data });

        socket.data.borrow_mut().close_cb = Some(Box::new(close_cb));

        let socket_weak_ref = Rc::downgrade(&socket);
        let func: Box<dyn Fn(SocketEventKind)> = Box::new(move |kind| {
            let socket = socket_weak_ref.upgrade().expect("Broken prerequisite");
            match kind {
                SocketEventKind::Read => {
                    socket.handle_read();
                },
                SocketEventKind::Write => {
                    socket.handle_write();
                },
                SocketEventKind::Event(events) => {
                    socket.handle_event(events);
                },
            }
        });
        let ctx = Box::new(CallbackContext {
            func: func,
            arg: (),
        });
        // move into pointer
        let ctx_ptr: *mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()> = Box::into_raw(ctx);

        // context free by pointer holder
        socket.data.borrow_mut().cb_ctx_ptr = Some(unsafe { NonNull::new_unchecked(ctx_ptr) });

        let base_ptr = socket.data.borrow().bufferevent.as_ptr();
        unsafe {
            bufferevent_setcb(
                base_ptr,
                Some(c_socket_read_cb),
                Some(c_socket_write_cb),
                Some(c_socket_event_cb),
                ctx_ptr as *mut _,
            )
        };
        unsafe {
            bufferevent_enable(
                socket.data.borrow().bufferevent.as_ptr(),
                (EV_WRITE | EV_READ) as i16
            )
        };

        socket
    }

    fn input_buffer(&self) -> SocketBufferRef {
        let base_ptr = self.data.borrow().bufferevent.as_ptr();
        let evbuffer: NonNull<evbuffer> = NonNull::new(unsafe {
            bufferevent_get_input(base_ptr)
        }).expect("event buffer pointer shoudn't be null");
        SocketBufferRef { evbuffer }
    }

    fn output_buffer(&self) -> SocketBufferRef {
        let base_ptr = self.data.borrow().bufferevent.as_ptr();
        let evbuffer: NonNull<evbuffer> = NonNull::new(unsafe {
            bufferevent_get_output(base_ptr)
        }).expect("event buffer pointer shoudn't be null");
        SocketBufferRef { evbuffer }
    }

    fn handle_read(&self) {
        let read_cb = self.data.borrow_mut().read_cb.clone();
        if let Some(read_cb) = read_cb {
            let in_buf = self.input_buffer();
            let res = read_cb(in_buf.remove_all_bytes());
            if let Err(err) = res {
                self.close_with_err(err);
            };
        };
    }

    fn handle_write(&self) {
        // currently nothing to do
    }

    fn handle_event(&self, events: i16) {
        let eof = (BEV_EVENT_EOF as i16 & events) != 0;
        let error = (BEV_EVENT_ERROR as i16 & events) != 0;
        let timeout = (BEV_EVENT_TIMEOUT as i16 & events) != 0;
        let connected = (BEV_EVENT_TIMEOUT as i16 & events) != 0;

        // at least one flag is on
        assert!(eof || error || timeout || connected);

        // at most one flag is on
        let count =
            if eof { 1 } else { 0 } +
            if error { 1 } else { 0 } +
            if timeout { 1 } else { 0 } +
            if connected { 1 } else { 0 };
        assert_eq!(count, 1);

        if eof {
            self.close();
        } else if error {
            self.close_with_err(EventError("Error event occurred in socket.".into()));
        } else if timeout {
            // currently to do nothing, we didn't use timeout yet
        } else if connected {
            // currently to do nothing
        }
    }

    fn on_data(&self, cb: impl Fn(Vec<u8>) -> Result<(), EventError> + 'static) -> Result<(), EventError> {
        let read_cb = self.data.borrow_mut().read_cb.clone();
        if let Some(_) = read_cb {
            return Err(EventError("Socket data handler already set".into()));
        };
        self.data.borrow_mut().read_cb = Some(Rc::new(cb));
        Ok(())
    }

    fn write(&self, bytes: Vec<u8>) -> Result<(), EventError> {
        let out_buf = self.output_buffer();
        out_buf.add_bytes(bytes)?;
        Ok(())
    }

    fn close_with_err(&self, err: EventError) {
        let close_cb = self.data.borrow_mut().close_cb.take();
        if let Some(close_cb) = close_cb {
            close_cb(self, Err(err));
        };
    }

    fn close(&self) {
        let close_cb = self.data.borrow_mut().close_cb.take();
        if let Some(close_cb) = close_cb {
            close_cb(self, Ok(()));
        };
    }
}

struct SocketBufferRef {
    evbuffer: NonNull<evbuffer>
}

impl SocketBufferRef {
    fn len(&self) -> usize {
        unsafe { evbuffer_get_length(self.evbuffer.as_ptr()) }
    }

    fn remove_all_bytes(&self) -> Vec<u8> {
        let len = self.len();
        let mut buffer = Vec::with_capacity(len);
        let buffer_ptr = buffer.as_mut_ptr();
        let reads = unsafe { evbuffer_remove(self.evbuffer.as_ptr(), buffer_ptr as *mut _, len) };
        assert_eq!(reads as usize, len);
        unsafe { buffer.set_len(len) };
        assert_eq!(self.len(), 0usize);
        return buffer;
    }

    fn add_bytes(&self, buffer: Vec<u8>) -> Result<(), EventError> {
        let len = buffer.len();
        let buffer_ptr = buffer.as_ptr();
        let ret = unsafe { evbuffer_add(self.evbuffer.as_ptr(), buffer_ptr as *const _, len) };
        if ret != 0 {
            return Err(EventError("Failed to write to socket output buffer".into()));
        };
        Ok(())
    }
}

struct CallbackContext<F, A> {
    func: F,
    arg: A,
}

fn main() {
    match try_main() {
        Err(EventError(message)) => eprintln!("Error: {}", message),
        _ => (),
    }
}

fn try_main() -> Result<(), EventError> {
    let port = 9995u16;
    let lp = EventLoop::try_new()?;
    let lp_weak_ref = Rc::downgrade(&lp);
    lp.bind_inet_port(port, move |fd| {
        let lp = lp_weak_ref.upgrade().expect("Broken prerequisite");
        let socket = lp.try_new_socket(fd)?;
        let socket_weak_ref = Rc::downgrade(&socket);
        socket.on_data(move |bytes| {
            let socket = socket_weak_ref.upgrade().expect("Broken prerequisite");

            println!("Received");
            socket.write(bytes)?;
            println!("Answered");
            Ok(())
        })?;
        Ok(())
    })?;

    let lp_weak_ref = Rc::downgrade(&lp);
    lp.handle_signal(SIGINT, move |_sig, _events| {
        let lp = lp_weak_ref.upgrade().expect("Broken prerequisite");

        println!("Caught an interrupt signal; exiting cleanly in two seconds.");
        lp.exit(2.0)?;
        Ok(())
    })?;
    lp.run()?;

    let last_err = lp.data.borrow().break_reason_err.clone();
    match last_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

extern "C" fn c_bind_cb(_listener: *mut evconnlistener, fd: i32, _sa: *mut sockaddr, _socklen: i32, ctx_ptr: *mut c_void) {
    let ctx: &mut CallbackContext<Box<dyn Fn(i32)>, ()> = unsafe {
        &mut *(ctx_ptr as *mut CallbackContext<Box<dyn Fn(i32)>, ()>)
    };
    (ctx.func)(fd);
}

extern "C" fn c_signal_cb(sig: i32, events: i16, ctx_ptr: *mut c_void) {
    let ctx: &mut CallbackContext<Box<dyn Fn(u32, i16)>, u32> = unsafe {
        &mut *(ctx_ptr as *mut CallbackContext<Box<dyn Fn(u32, i16)>, u32>)
    };
    assert!(ctx.arg == sig as u32);
    (ctx.func)(ctx.arg, events);
}

extern "C" fn c_socket_read_cb(_bev: *mut bufferevent, ctx_ptr: *mut c_void) {
    let ctx: &mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()> = unsafe {
        &mut *(ctx_ptr as *mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()>)
    };
    (ctx.func)(SocketEventKind::Read);
}

extern "C" fn c_socket_write_cb(_bev: *mut bufferevent, ctx_ptr: *mut c_void) {
    let ctx: &mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()> = unsafe {
        &mut *(ctx_ptr as *mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()>)
    };
    (ctx.func)(SocketEventKind::Write);
}

extern "C" fn c_socket_event_cb(_bev: *mut bufferevent, events: i16, ctx_ptr: *mut c_void) {
    let ctx: &mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()> = unsafe {
        &mut *(ctx_ptr as *mut CallbackContext<Box<dyn Fn(SocketEventKind)>, ()>)
    };
    (ctx.func)(SocketEventKind::Event(events));
}
