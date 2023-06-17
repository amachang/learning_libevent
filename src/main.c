#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <signal.h>

#include <event2/bufferevent.h>
#include <event2/buffer.h>
#include <event2/listener.h>
#include <event2/util.h>
#include <event2/event.h>

static const unsigned short PORT = 9995;

static void listener_cb(struct evconnlistener *, evutil_socket_t,
        struct sockaddr *, int socklen, void *);
static void conn_writecb(struct bufferevent *, void *);
static void conn_readcb(struct bufferevent *, void *);
static void conn_eventcb(struct bufferevent *, short, void *);
static void signal_cb(evutil_socket_t, short, void *);

int main(int argc, char **argv)
{
    struct event_base *base = event_base_new();
    if (!base) {
        fprintf(stderr, "Could not initialize libevent!\n");
        return 1;
    }

    struct sockaddr_in sin = {0};
    sin.sin_family = AF_INET;
    sin.sin_port = htons(PORT);

    struct evconnlistener *listener = evconnlistener_new_bind(base, listener_cb, (void *)base,
            LEV_OPT_REUSEABLE|LEV_OPT_CLOSE_ON_FREE, -1,
            (struct sockaddr*)&sin,
            sizeof(sin));

    if (!listener) {
        fprintf(stderr, "Could not create a listener!\n");
        return 1;
    }

    struct event *signal_event = evsignal_new(base, SIGINT, signal_cb, (void *)base);

    if (!signal_event || event_add(signal_event, NULL)<0) {
        fprintf(stderr, "Could not create/add a signal event!\n");
        return 1;
    }

    printf("Start listening the port: %d\n", PORT);
    event_base_dispatch(base);

    evconnlistener_free(listener);
    event_free(signal_event);
    event_base_free(base);

    printf("done\n");
    return 0;
}

static void listener_cb(struct evconnlistener *listener, evutil_socket_t fd,
        struct sockaddr *sa, int socklen, void *user_data)
{
    struct event_base *base = user_data;
    struct bufferevent *bev = bufferevent_socket_new(base, fd, BEV_OPT_CLOSE_ON_FREE);
    if (!bev) {
        fprintf(stderr, "Error constructing bufferevent!");
        event_base_loopbreak(base);
        return;
    }
    bufferevent_setcb(bev, conn_readcb, conn_writecb, conn_eventcb, NULL);
    bufferevent_enable(bev, EV_WRITE | EV_READ);
}

static void conn_writecb(struct bufferevent *bev, void *user_data)
{
    struct evbuffer *output = bufferevent_get_output(bev);
    if (evbuffer_get_length(output) == 0) {
        printf("Answered\n");
    }
}

static void conn_readcb(struct bufferevent *bev, void *user_data)
{
    struct evbuffer* evbuf_in = bufferevent_get_input(bev);
    struct evbuffer* evbuf_out = bufferevent_get_output(bev);
    char buf[1000];
    while (1) {
        size_t inputs = evbuffer_get_length(evbuf_in);
        size_t writes = evbuffer_remove_buffer(evbuf_in, evbuf_out, inputs);
        if (inputs <= writes) {
            break;
        }
    }
    printf("Received\n");
}

static void conn_eventcb(struct bufferevent *bev, short events, void *user_data)
{

    if (events & BEV_EVENT_READING) {
        printf("Error encountered while reading: %s\n", strerror(errno));
    }

    if (events & BEV_EVENT_WRITING) {
        printf("Error encountered while writing: %s\n", strerror(errno));
    }

    if (events & BEV_EVENT_EOF) {
        printf("Eof reached.\n");
    }

    if (events & BEV_EVENT_ERROR) {
        printf("Unrecoverable error encountered: %s\n", strerror(errno));
    }

    if (events & BEV_EVENT_TIMEOUT) {
        printf("User-specified timeout reached.\n");
    }

    if (events & BEV_EVENT_CONNECTED) {
        printf("Connect operation finished.\n");
    }


    /* None of the other events can happen here, since we haven't enabled
     * timeouts */
    bufferevent_free(bev);
}

static void signal_cb(evutil_socket_t sig, short events, void *user_data)
{
    struct event_base *base = user_data;
    struct timeval delay = { 2, 0 };

    printf("Caught an interrupt signal; exiting cleanly in two seconds.\n");

    event_base_loopexit(base, &delay);
}

