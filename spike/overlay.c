/* M0-03: прототип оверлея.
 *
 * - override_redirect (WM не управляет, нет в Alt+Tab и на панели задач)
 * - ARGB visual (прозрачность через композитор)
 * - пустая input shape (клики проходят сквозь)
 * - не забирает фокус
 *
 * build: gcc -O2 -o overlay overlay.c -lX11 -lXrender -lXext
 * run:   ./overlay [seconds] [EN|RU]
 */
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/Xrender.h>
#include <X11/extensions/shape.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define W 88
#define H 44

int main(int argc, char **argv) {
    int seconds = argc > 1 ? atoi(argv[1]) : 8;
    const char *text = argc > 2 ? argv[2] : "EN";

    Display *dpy = XOpenDisplay(NULL);
    if (!dpy) { fprintf(stderr, "cannot open display\n"); return 1; }
    int screen = DefaultScreen(dpy);
    Window root = RootWindow(dpy, screen);

    /* Есть ли композитор — без него прозрачность не сработает. */
    Atom cm_atom = XInternAtom(dpy, "_NET_WM_CM_S0", False);
    printf("compositor: %s\n",
           XGetSelectionOwner(dpy, cm_atom) != None ? "yes" : "NO (alpha won't blend)");

    XVisualInfo vinfo;
    if (!XMatchVisualInfo(dpy, screen, 32, TrueColor, &vinfo)) {
        fprintf(stderr, "no 32-bit ARGB visual\n");
        return 2;
    }

    XSetWindowAttributes attrs = {0};
    attrs.colormap = XCreateColormap(dpy, root, vinfo.visual, AllocNone);
    attrs.background_pixel = 0;
    attrs.border_pixel = 0;
    attrs.override_redirect = True;           /* WM не управляет окном */
    attrs.event_mask = ExposureMask;

    /* Позиция — рядом с курсором мыши. */
    Window rr, cr; int rx, ry, wx, wy; unsigned mask;
    XQueryPointer(dpy, root, &rr, &cr, &rx, &ry, &wx, &wy, &mask);
    int x = rx + 14, y = ry - H - 24;
    if (x + W > DisplayWidth(dpy, screen)) x = rx - W - 14;
    if (y < 0) y = ry + 24;

    Window win = XCreateWindow(dpy, root, x, y, W, H, 0,
                               vinfo.depth, InputOutput, vinfo.visual,
                               CWColormap | CWBackPixel | CWBorderPixel |
                               CWOverrideRedirect | CWEventMask, &attrs);
    if (!win) { fprintf(stderr, "XCreateWindow failed\n"); return 3; }

    /* Пустая область ввода => клики проходят сквозь окно. */
    XShapeCombineRectangles(dpy, win, ShapeInput, 0, 0, NULL, 0, ShapeSet, Unsorted);

    XMapRaised(dpy, win);
    XFlush(dpy);

    XRenderPictFormat *fmt = XRenderFindVisualFormat(dpy, vinfo.visual);
    Picture pic = XRenderCreatePicture(dpy, win, fmt, 0, NULL);

    /* Фон окна — полностью прозрачный. */
    XRenderColor transparent = {0, 0, 0, 0};
    XRenderFillRectangle(dpy, PictOpSrc, pic, &transparent, 0, 0, W, H);

    /* Плашка: #1b1c1d при alpha 0xe0 (premultiplied, 16-bit). */
    XRenderColor bg = {6095, 6317, 6542, 57536};
    XRenderFillRectangle(dpy, PictOpSrc, pic, &bg, 0, 0, W, H);

    /* Текст — core font (в проде будет нормальный шейпинг). */
    XFontStruct *font = XLoadQueryFont(dpy, "fixed");
    GC gc = XCreateGC(dpy, win, 0, NULL);
    if (font) XSetFont(dpy, gc, font->fid);
    XSetForeground(dpy, gc, 0xFFFFFFFF);      /* непрозрачный белый */
    int tw = font ? XTextWidth(font, text, (int)strlen(text)) : 12;
    int th = font ? font->ascent + font->descent : 12;
    XDrawString(dpy, win, gc, (W - tw) / 2, (H + th) / 2 - (font ? font->descent : 0),
                text, (int)strlen(text));
    XFlush(dpy);

    printf("overlay win=0x%lx at (%d,%d) size %dx%d text=%s, %d s\n",
           win, x, y, W, H, text, seconds);
    printf("проверьте: (1) виден поверх окон, (2) клик проходит сквозь,\n");
    printf("           (3) нет в Alt+Tab/панели задач, (4) фокус не перехвачен\n");
    fflush(stdout);

    /* Ждём и обрабатываем Expose, чтобы окно не затиралось. */
    time_t end = time(NULL) + seconds;
    while (time(NULL) < end) {
        while (XPending(dpy)) {
            XEvent ev; XNextEvent(dpy, &ev);
            if (ev.type == Expose) {
                XRenderFillRectangle(dpy, PictOpSrc, pic, &transparent, 0, 0, W, H);
                XRenderFillRectangle(dpy, PictOpSrc, pic, &bg, 0, 0, W, H);
                XDrawString(dpy, win, gc, (W - tw) / 2,
                            (H + th) / 2 - (font ? font->descent : 0),
                            text, (int)strlen(text));
                XFlush(dpy);
            }
        }
        usleep(50000);
    }

    XDestroyWindow(dpy, win);
    XCloseDisplay(dpy);
    printf("overlay closed\n");
    return 0;
}
