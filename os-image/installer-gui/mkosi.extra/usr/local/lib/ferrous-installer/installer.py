#!/usr/bin/python3
"""FerrousNAS graphical installer.

A step-by-step wizard in the style of Calamares: Welcome, Disk, Account,
Summary, Install, Finish. It runs full-screen under the cage compositor,
chrooted into the GUI layer (gui.sqfs) by ferrous-gui, and does no install
work itself: it drives the installer system's `ferrous-install` (reachable
at /host) non-interactively, so the graphical and text installers perform
exactly the same steps.

On exit it writes what to do next ("reboot", "poweroff" or "text") to
RESULT for ferrous-gui to act on.
"""
import os
import re
import subprocess
import sys
import threading

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, GLib, Gtk  # noqa: E402

HOST = "/host"
MEDIA = HOST + "/run/ferrous-media"
RESULT = "/tmp/ferrous-result"
INSTALL = ["chroot", HOST, "/usr/local/sbin/ferrous-install"]

USERNAME_RE = re.compile(r"^[a-z_][a-z0-9_-]{0,31}$")
RESERVED_USERS = {"root", "ferrous", "nobody", "daemon", "bin", "sys"}
HOSTNAME_RE = re.compile(r"^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$")

STEPS = ["Welcome", "Disk", "Account", "Summary", "Install", "Finish"]

CSS = """
@define-color accent_bg_color #b7410e;
@define-color accent_color #b7410e;
.sidebar-pane { background-color: #24272e; color: #e8e8e8; }
.brand { font-size: 22px; font-weight: 800; color: #ffffff; }
.brand-sub { color: #a0a4ad; font-size: 12px; }
.step { padding: 10px 16px; border-radius: 8px; color: #a0a4ad; font-size: 15px; }
.step.done { color: #d0d3d9; }
.step.current { background-color: #b7410e; color: #ffffff; font-weight: 700; }
.page-title { font-size: 28px; font-weight: 800; }
.footer { padding: 12px 24px; border-top: 1px solid alpha(currentColor, 0.12); }
.warning-box { background-color: alpha(#e01b24, 0.10); border-radius: 10px; padding: 12px 16px; }
"""


def human(n):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1000 or unit == "TB":
            return f"{n:.0f} {unit}" if unit in ("B", "KB") else f"{n:.1f} {unit}"
        n /= 1000


def read_text(path, default=""):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def list_disks():
    """[(device, size_bytes, model)] of the disks FerrousNAS may go on."""
    out = subprocess.run(INSTALL + ["--list-disks"], capture_output=True, text=True).stdout
    disks = []
    for line in out.splitlines():
        parts = line.split("\t")
        if len(parts) < 2 or not parts[0].startswith("/dev/"):
            continue
        name = parts[0]
        sectors = read_text(f"{HOST}/sys/block/{os.path.basename(name)}/size", "0")
        disks.append((name, int(sectors) * 512, parts[2].strip() if len(parts) > 2 else ""))
    return disks


def page_box(title, subtitle=None):
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
    box.set_margin_top(40)
    box.set_margin_bottom(24)
    box.set_margin_start(48)
    box.set_margin_end(48)
    t = Gtk.Label(label=title, xalign=0)
    t.add_css_class("page-title")
    box.append(t)
    if subtitle:
        s = Gtk.Label(label=subtitle, xalign=0, wrap=True)
        s.add_css_class("dim-label")
        box.append(s)
    return box


class Installer(Adw.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="FerrousNAS Installer")
        self.set_default_size(1024, 700)
        self.version = read_text(f"{MEDIA}/VERSION", "")
        self.image_size = int(read_text(f"{MEDIA}/ferrous-nas.size", "0") or 0)
        self.disk = None
        self.step = 0
        self.installing = False
        self.failed = False

        root = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        root.append(self.build_sidebar())

        main = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, hexpand=True)
        self.stack = Gtk.Stack(vexpand=True, transition_type=Gtk.StackTransitionType.SLIDE_LEFT_RIGHT)
        self.stack.add_named(self.build_welcome(), "Welcome")
        self.stack.add_named(self.build_disk(), "Disk")
        self.stack.add_named(self.build_account(), "Account")
        self.stack.add_named(self.build_summary(), "Summary")
        self.stack.add_named(self.build_progress(), "Install")
        self.stack.add_named(self.build_finish(), "Finish")
        main.append(self.stack)
        main.append(self.build_footer())
        root.append(main)

        self.set_content(root)
        self.show_step(0)

    # ---- chrome -----------------------------------------------------------

    def build_sidebar(self):
        side = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        side.add_css_class("sidebar-pane")
        side.set_size_request(240, -1)
        side.set_margin_top(0)
        inner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        inner.set_margin_top(32)
        inner.set_margin_start(16)
        inner.set_margin_end(16)
        inner.set_margin_bottom(16)
        inner.set_vexpand(True)
        brand = Gtk.Label(label="FerrousNAS", xalign=0)
        brand.add_css_class("brand")
        inner.append(brand)
        sub = Gtk.Label(label=f"Installer {self.version}".strip(), xalign=0)
        sub.add_css_class("brand-sub")
        sub.set_margin_bottom(24)
        inner.append(sub)
        self.step_labels = []
        for name in STEPS:
            lbl = Gtk.Label(label=name, xalign=0)
            lbl.add_css_class("step")
            inner.append(lbl)
            self.step_labels.append(lbl)
        spacer = Gtk.Box(vexpand=True)
        inner.append(spacer)
        self.power_btn = Gtk.Button(label="Power off")
        self.power_btn.add_css_class("flat")
        self.power_btn.connect("clicked", lambda *_: self.finish("poweroff"))
        inner.append(self.power_btn)
        side.append(inner)
        return side

    def build_footer(self):
        bar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        bar.add_css_class("footer")
        self.text_btn = Gtk.Button(label="Use text installer")
        self.text_btn.add_css_class("flat")
        self.text_btn.connect("clicked", lambda *_: self.finish("text"))
        bar.append(self.text_btn)
        bar.append(Gtk.Box(hexpand=True))
        self.back_btn = Gtk.Button(label="Back")
        self.back_btn.connect("clicked", self.on_back)
        bar.append(self.back_btn)
        self.next_btn = Gtk.Button(label="Next")
        self.next_btn.add_css_class("suggested-action")
        self.next_btn.connect("clicked", self.on_next)
        bar.append(self.next_btn)
        return bar

    def show_step(self, i):
        self.step = i
        name = STEPS[i]
        self.stack.set_visible_child_name(name)
        for j, lbl in enumerate(self.step_labels):
            for c in ("current", "done"):
                lbl.remove_css_class(c)
            if j == i:
                lbl.add_css_class("current")
            elif j < i:
                lbl.add_css_class("done")
        self.back_btn.set_visible(0 < i < 4 or (i == 5 and self.failed))
        self.back_btn.set_label("Start over" if self.failed else "Back")
        self.text_btn.set_visible(i < 4 or self.failed)
        self.power_btn.set_sensitive(i != 4)
        self.next_btn.remove_css_class("destructive-action")
        self.next_btn.add_css_class("suggested-action")
        self.next_btn.set_visible(i != 4)
        self.next_btn.set_label({3: "Install", 5: "Reboot"}.get(i, "Next"))
        if i == 3:
            self.next_btn.remove_css_class("suggested-action")
            self.next_btn.add_css_class("destructive-action")
            self.fill_summary()
        if i == 1:
            self.refresh_disks()
        self.validate()

    def validate(self, *_):
        ok = True
        if self.step == 1:
            ok = self.disk is not None
        elif self.step == 2:
            ok = self.account_error() is None
            err = self.account_error() if (self.user_row.get_text() or self.pw_row.get_text()) else None
            self.account_msg.set_label(err or "")
        self.next_btn.set_sensitive(ok)

    def on_back(self, *_):
        if self.failed:
            self.failed = False
            self.finish_title.set_label("All done!")
            self.show_step(1)
        else:
            self.show_step(self.step - 1)

    def on_next(self, *_):
        if self.step == 3:
            self.confirm_install()
        elif self.step == 5:
            self.finish("reboot")
        else:
            self.show_step(self.step + 1)

    def finish(self, what):
        try:
            with open(RESULT, "w") as f:
                f.write(what + "\n")
        except OSError:
            pass
        self.get_application().quit()

    # ---- pages ------------------------------------------------------------

    def build_welcome(self):
        box = page_box("Welcome to FerrousNAS",
                       "This will install FerrousNAS, a self-hosted NAS appliance, onto a disk "
                       "in this computer.")
        group = Adw.PreferencesGroup()
        for icon, title, sub in (
            ("drive-harddisk-symbolic", "Pick a disk", "FerrousNAS takes over the whole disk you choose."),
            ("avatar-default-symbolic", "Create your login", "A user for the console and SSH, with sudo."),
            ("network-wired-symbolic", "Open the dashboard",
             "After reboot, browse to https://<this machine's IP>:4200 to finish setup."),
        ):
            row = Adw.ActionRow(title=title, subtitle=GLib.markup_escape_text(sub))
            row.add_prefix(Gtk.Image.new_from_icon_name(icon))
            group.add(row)
        box.append(group)
        return box

    def build_disk(self):
        box = page_box("Choose a disk", "FerrousNAS needs a whole disk. The installation medium is not listed.")
        warn = Gtk.Label(label="⚠  Everything on the selected disk will be erased.", xalign=0, wrap=True)
        warn.add_css_class("warning-box")
        box.append(warn)
        self.disk_group = Adw.PreferencesGroup()
        refresh = Gtk.Button(icon_name="view-refresh-symbolic", tooltip_text="Rescan disks")
        refresh.add_css_class("flat")
        refresh.connect("clicked", lambda *_: self.refresh_disks())
        self.disk_group.set_header_suffix(refresh)
        self.disk_rows = []
        scroll = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER)
        scroll.set_child(self.disk_group)
        box.append(scroll)
        return box

    def refresh_disks(self):
        for row in self.disk_rows:
            self.disk_group.remove(row)
        self.disk_rows = []
        disks = list_disks()
        if not disks:
            row = Adw.ActionRow(title="No disks found", subtitle="Connect a disk, then rescan.")
            self.disk_group.add(row)
            self.disk_rows.append(row)
        first = None
        for name, size, model in disks:
            check = Gtk.CheckButton(valign=Gtk.Align.CENTER)
            if first is None:
                first = check
            else:
                check.set_group(first)
            big_enough = size >= self.image_size
            sub = human(size) + (f" · {GLib.markup_escape_text(model)}" if model else "")
            if not big_enough:
                sub += f" · too small (needs {human(self.image_size)})"
            row = Adw.ActionRow(title=name, subtitle=sub, activatable_widget=check)
            row.add_prefix(check)
            row.add_suffix(Gtk.Image.new_from_icon_name("drive-harddisk-symbolic"))
            row.set_sensitive(big_enough)
            check.set_active(name == self.disk and big_enough)
            check.connect("toggled", self.on_disk_toggled, name)
            self.disk_group.add(row)
            self.disk_rows.append(row)
        if self.disk not in [d[0] for d in disks if d[1] >= self.image_size]:
            self.disk = None
        self.validate()

    def on_disk_toggled(self, check, name):
        if check.get_active():
            self.disk = name
        self.validate()

    def build_account(self):
        box = page_box("Create your login",
                       "This account logs in at the console and over SSH, and can use sudo. "
                       "The dashboard's admin account is created separately, in the dashboard.")
        group = Adw.PreferencesGroup()
        self.user_row = Adw.EntryRow(title="Username")
        self.pw_row = Adw.PasswordEntryRow(title="Password")
        self.pw2_row = Adw.PasswordEntryRow(title="Repeat password")
        self.host_row = Adw.EntryRow(title="Hostname")
        self.host_row.set_text("ferrous-nas")
        for r in (self.user_row, self.pw_row, self.pw2_row, self.host_row):
            r.connect("changed", self.validate)
            group.add(r)
        box.append(group)
        self.account_msg = Gtk.Label(xalign=0, wrap=True)
        self.account_msg.add_css_class("error")
        box.append(self.account_msg)
        return box

    def account_error(self):
        user = self.user_row.get_text()
        if not USERNAME_RE.match(user) or user in RESERVED_USERS:
            return ("Username: lowercase letters, digits, - or _, starting with a letter "
                    "(not root or ferrous).")
        if not self.pw_row.get_text():
            return "Choose a password."
        if self.pw_row.get_text() != self.pw2_row.get_text():
            return "The passwords don't match."
        if not HOSTNAME_RE.match(self.host_row.get_text()):
            return "Hostname: letters, digits and -, up to 63 characters."
        return None

    def build_summary(self):
        box = page_box("Summary", "Check everything before installing. Nothing has been changed yet.")
        group = Adw.PreferencesGroup()
        self.sum_disk = Adw.ActionRow(title="Install to")
        self.sum_user = Adw.ActionRow(title="Login")
        self.sum_host = Adw.ActionRow(title="Hostname")
        for r in (self.sum_disk, self.sum_user, self.sum_host):
            r.add_css_class("property")
            group.add(r)
        box.append(group)
        warn = Gtk.Label(xalign=0, wrap=True)
        warn.add_css_class("warning-box")
        self.sum_warn = warn
        box.append(warn)
        return box

    def fill_summary(self):
        size = next((s for n, s, _ in list_disks() if n == self.disk), 0)
        self.sum_disk.set_subtitle(f"{self.disk} ({human(size)})")
        self.sum_user.set_subtitle(self.user_row.get_text())
        self.sum_host.set_subtitle(self.host_row.get_text())
        self.sum_warn.set_label(f"⚠  All data on {self.disk} will be permanently erased.")

    def confirm_install(self):
        dlg = Adw.AlertDialog(heading=f"Erase {self.disk}?",
                              body="FerrousNAS will be installed and everything on this disk will be lost.")
        dlg.add_response("cancel", "Cancel")
        dlg.add_response("install", "Erase and install")
        dlg.set_response_appearance("install", Adw.ResponseAppearance.DESTRUCTIVE)
        dlg.set_default_response("cancel")
        dlg.connect("response", lambda _d, r: r == "install" and self.start_install())
        dlg.present(self)

    def build_progress(self):
        box = page_box("Installing", "This takes a few minutes. Don't turn the computer off.")
        self.progress = Gtk.ProgressBar(show_text=True)
        self.progress.set_margin_top(24)
        box.append(self.progress)
        self.progress_label = Gtk.Label(xalign=0)
        self.progress_label.add_css_class("dim-label")
        box.append(self.progress_label)
        return box

    def build_finish(self):
        self.finish_box = page_box("All done!")
        self.finish_title = self.finish_box.get_first_child()
        self.finish_text = Gtk.Label(xalign=0, wrap=True, use_markup=True)
        self.finish_box.append(self.finish_text)
        return self.finish_box

    # ---- install ----------------------------------------------------------

    def start_install(self):
        self.show_step(4)
        self.installing = True
        user, pw, host = self.user_row.get_text(), self.pw_row.get_text(), self.host_row.get_text()
        threading.Thread(target=self.run_install, args=(self.disk, user, pw, host), daemon=True).start()

    def set_progress(self, frac, text=None):
        self.progress.set_fraction(max(0.0, min(1.0, frac)))
        if text is not None:
            self.progress_label.set_label(text)
        return False

    def run_install(self, disk, user, pw, host):
        # Weights of each ferrous-install step on the progress bar.
        stages = {"Wiping": (0.0, 0.02), "Writing": (0.02, 0.90),
                  "Growing": (0.90, 0.97), "Setting": (0.97, 1.0)}
        cur = (0.0, 0.02)
        error = None
        proc = subprocess.Popen(INSTALL + ["--install", disk], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        proc.stdin.write(f"{user}\n{pw}\n{host}\n")
        proc.stdin.close()
        last_lines = []
        for line in proc.stdout:
            line = line.rstrip("\n")
            if line.startswith("STEP "):
                text = line[5:]
                cur = stages.get(text.split()[0], cur)
                GLib.idle_add(self.set_progress, cur[0], text + "…")
            elif line.startswith("BYTES ") and self.image_size:
                done = int(line.split()[1])
                f = cur[0] + (cur[1] - cur[0]) * done / self.image_size
                GLib.idle_add(self.set_progress, f, f"Writing FerrousNAS to {disk}… "
                              f"{human(done)} of {human(self.image_size)}")
            elif line.startswith("ERROR "):
                error = line[6:]
            elif line == "DONE":
                GLib.idle_add(self.set_progress, 1.0, "Finished.")
            else:
                last_lines = (last_lines + [line])[-5:]
        rc = proc.wait()
        if rc != 0 and not error:
            error = "\n".join(last_lines) or f"ferrous-install exited with status {rc}"
        GLib.idle_add(self.install_done, disk, user, error)

    def install_done(self, disk, user, error):
        self.installing = False
        if error:
            self.failed = True
            self.finish_title.set_label("Installation failed")
            self.finish_text.set_label(
                f"<b>{GLib.markup_escape_text(error)}</b>\n\nYou can start over, try the "
                "text installer, or reboot.")
            self.show_step(5)
        else:
            self.finish_text.set_label(
                f"FerrousNAS is installed on <b>{GLib.markup_escape_text(disk)}</b>.\n\n"
                f"1.  Remove the installation medium.\n"
                f"2.  Reboot. Log in at the console or over SSH as <b>{GLib.markup_escape_text(user)}</b>.\n"
                f"3.  Browse to <tt>https://&lt;this machine's IP&gt;:4200</tt> to create the dashboard admin.")
            self.show_step(5)
        return False


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id="org.ferrousnas.Installer")

    def do_activate(self):
        css = Gtk.CssProvider()
        css.load_from_string(CSS)
        from gi.repository import Gdk
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css,
                                                  Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        win = Installer(self)
        win.present()


if __name__ == "__main__":
    sys.exit(App().run([]))
