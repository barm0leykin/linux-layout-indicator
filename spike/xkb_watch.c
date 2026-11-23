/* M0-01 / M0-02: проверка событийного детекта смены раскладки.
 *
 * Собирает XkbStateNotify, печатает группу при каждом переключении.
 * Ничего не перехватывает (нет XGrabKey), ввод не затрагивает.
 *
 * build: gcc -O2 -o xkb_watch xkb_watch.c -lX11
 * run:   ./xkb_watch [seconds]     (по умолчанию 30)
 */
#include <X11/Xlib.h>
#include <X11/XKBlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000.0 + ts.tv_nsec / 1e6;
}

int main(int argc, char **argv) {
    int seconds = argc > 1 ? atoi(argv[1]) : 30;

    Display *dpy = XOpenDisplay(NULL);
    if (!dpy) { fprintf(stderr, "cannot open display\n"); return 1; }

    int opcode, eventBase, errorBase;
    int major = XkbMajorVersion, minor = XkbMinorVersion;
    if (!XkbQueryExtension(dpy, &opcode, &eventBase, &errorBase, &major, &minor)) {
        fprintf(stderr, "XKB extension not available\n");
        return 2;
    }
    printf("XKB %d.%d  opcode=%d eventBase=%d\n", major, minor, opcode, eventBase);

    XkbStateRec state;
    if (XkbGetState(dpy, XkbUseCoreKbd, &state) == Success)
        printf("initial: group=%d locked_group=%d\n", state.group, state.locked_group);

    /* Какие имена групп знает сам X-сервер (для сравнения с setxkbmap -query). */
    XkbDescPtr xkb = XkbGetMap(dpy, 0, XkbUseCoreKbd);
    if (xkb) {
        if (XkbGetNames(dpy, XkbGroupNamesMask, xkb) == Success) {
            for (int i = 0; i < XkbNumKbdGroups; i++) {
                Atom a = xkb->names->groups[i];
                if (a != None) {
                    char *s = XGetAtomName(dpy, a);
                    printf("xkb group_name[%d] = %s\n", i, s ? s : "?");
                    if (s) XFree(s);
                }
            }
        }
        XkbFreeKeyboard(xkb, 0, True);
    }

    /* Подписка ТОЛЬКО на смену группы. */
    XkbSelectEventDetails(dpy, XkbUseCoreKbd, XkbStateNotify,
                          XkbGroupStateMask, XkbGroupStateMask);
    XFlush(dpy);

    printf("watching %d s — переключите раскладку (Alt+Shift)\n", seconds);
    fflush(stdout);

    double t0 = now_ms();
    int count = 0;
    while ((now_ms() - t0) < seconds * 1000.0) {
        while (XPending(dpy)) {
            XEvent ev;
            XNextEvent(dpy, &ev);
            if (ev.type == eventBase + XkbEventCode) {
                XkbEvent *xkbev = (XkbEvent *)&ev;
                if (xkbev->any.xkb_type == XkbStateNotify) {
                    printf("+%8.3f ms  StateNotify  group=%d  locked=%d  changed=0x%x\n",
                           now_ms() - t0,
                           xkbev->state.group,
                           xkbev->state.locked_group,
                           xkbev->state.changed);
                    fflush(stdout);
                    count++;
                }
            }
        }
        usleep(5000);
    }

    printf("done: %d events\n", count);
    XCloseDisplay(dpy);
    return 0;
}
