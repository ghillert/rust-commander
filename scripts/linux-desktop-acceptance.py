#!/usr/bin/env python3
"""Real X11 Nautilus/Dolphin copy drags and AT-SPI/Orca smoke acceptance.

Run as an unprivileged user under xvfb-run and dbus-run-session. Requires the
file managers, openbox, xdotool, python3-pyatspi, Orca and GTK3 introspection.
Only private temporary fixture files/configuration are used. No user files move.
This records automation evidence, not human assessment of audible speech.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    if os.geteuid()==0:
        raise RuntimeError("Run this isolated desktop fixture as an unprivileged user; Dolphin refuses root")
    import pyatspi
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manager",action="append",choices=["nautilus","dolphin"])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    os.environ["QT_LINUX_ACCESSIBILITY_ALWAYS_ON"]="1"
    children, logs = [], []

    def launch(command, name, env=None):
        log = (args.output / (name + ".log")).open("w")
        logs.append(log)
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        children.append(child)
        return child

    def run(*command):
        return subprocess.check_output(command, text=True).strip()

    def wait(check, label, seconds=25):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                value = check()
                if value:
                    return value
            except (RuntimeError, LookupError, subprocess.CalledProcessError):
                pass
            time.sleep(.15)
        raise RuntimeError("Timed out: " + label)

    def walk(node, depth=0):
        if depth > 24:
            return
        yield node
        try:
            for child in node:
                if child is not None:
                    yield from walk(child, depth + 1)
        except (LookupError, RuntimeError):
            pass

    def named(name, role=None):
        for node in walk(pyatspi.Registry.getDesktop(0)):
            try:
                if node.name.replace("&", "").split("\t")[0].strip() == name and (role is None or node.getRoleName() == role):
                    rect = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    if rect.width > 0 and rect.height > 0:
                        return node
            except (LookupError, RuntimeError, NotImplementedError):
                pass

    def center(node, native_window=None, gtk=False):
        r = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        x,y=r.x+r.width//2,r.y+r.height//2
        # GTK4 exposes window-local positions even for SCREEN_COORDS on X11.
        # Qt/AccessKit already return screen coordinates; translate only GTK.
        if native_window and gtk:
            geometry=dict(line.split("=",1) for line in run("xdotool","getwindowgeometry","--shell",native_window).splitlines() if "=" in line)
            x+=int(geometry["X"]);y+=int(geometry["Y"])
        print(f"AT-SPI {node.name!r}: {x},{y}",flush=True)
        return x,y

    def capture(name):
        import gi
        gi.require_version("Gdk", "3.0")
        from gi.repository import Gdk
        screen=Gdk.get_default_root_window()
        Gdk.pixbuf_get_from_window(screen,0,0,screen.get_width(),screen.get_height()).savev(str(args.output/name),"png",[],[])

    def click(node):
        x, y = center(node)
        run("xdotool", "mousemove", str(x), str(y), "click", "1")

    def drag(start, end):
        x, y = start
        run("xdotool", "mousemove", str(x), str(y), "click", "1")
        time.sleep(.65)
        run("xdotool", "mousedown", "1")
        try:
            for i in range(1, 31):
                if i==4:run("xdotool","keydown","Control_L")
                run("xdotool", "mousemove", str(x + (end[0]-x)*i//30), str(y + (end[1]-y)*i//30))
                time.sleep(.045)
            time.sleep(.5)
        finally:
            run("xdotool", "mouseup", "1")
            time.sleep(.5)
            run("xdotool","keyup","Control_L")

    def window(pid, x, width):
        def find():
            for line in run("wmctrl", "-lp").splitlines():
                parts = line.split(None, 4)
                if len(parts) >= 4 and parts[2] == str(pid):
                    return parts[0]
        wid = wait(find, "window for PID " + str(pid))
        run("wmctrl", "-ir", wid, "-b", "remove,maximized_vert,maximized_horz")
        run("wmctrl", "-ir", wid, "-e", f"0,{x},0,{width},740")
        return wid

    try:
        launch(["openbox"], "openbox")
        wait(lambda: "Openbox" in run("wmctrl", "-m"), "window manager")
        run("gsettings", "set", "org.gnome.desktop.interface", "toolkit-accessibility", "true")
        launch(["orca", "--replace", "--debug", "--debug-file=" + str(args.output / "orca-debug.log")], "orca")
        # Querying AT-SPI activates AccessKit's bridge before enumerating rows.
        pyatspi.Registry.getDesktop(0)
        with tempfile.TemporaryDirectory(prefix="rust-commander-desktop-") as temp:
            root = Path(temp)
            left, right, config = (root / p for p in ["left", "right", "config"])
            for folder in [left, right, config]:
                folder.mkdir()
            for manager in ["nautilus", "dolphin"]:
                (left / (manager + "-outgoing.txt")).write_text(manager + " outgoing payload")
            (config / "settings.json").write_text(json.dumps({"preferences": {"sidebar": False, "contrast": "High"}}))
            env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-software", ACCESSKIT_UNIX_ALWAYS_ENABLED="1", RUST_COMMANDER_DND_TRACE="1")
            app = launch([str(args.binary), str(left), str(right)], "rust-commander", env)
            app_window = window(app.pid, 0, 900)
            wait(lambda: named("Left files"), "Rust Commander AT-SPI file list")
            wait(lambda: named("Right files"), "Rust Commander AT-SPI other file list")
            evidence = []
            for manager in args.manager or ["nautilus", "dolphin"]:
                folder = root / manager
                folder.mkdir()
                incoming = manager + "-incoming.txt"
                outgoing = manager + "-outgoing.txt"
                (folder / incoming).write_text(manager + " incoming payload")
                process = launch([manager, "--new-window", str(folder)], manager)
                manager_window=window(process.pid, 950, 620)
                wait(lambda:named(incoming),manager+" directory content")
                time.sleep(.5)
                source = wait(lambda: named(outgoing), outgoing)
                capture(manager+"-before-outgoing.png")
                drag(center(source), (1250, 540))
                time.sleep(1)
                capture(manager+"-after-outgoing.png")
                if manager=="dolphin" and not (folder/outgoing).exists():
                    # Dolphin presents its normal Move/Copy/Link choice for a
                    # drag without a persistent keyboard modifier at release.
                    print("Dolphin drop actions:",[(n.name,n.getRoleName()) for n in walk(pyatspi.Registry.getDesktop(0)) if "here" in n.name.lower()],flush=True)
                    copy_here=wait(lambda:named("Copy Here"),"Dolphin Copy Here")
                    click(copy_here)
                wait(lambda: (folder / outgoing).is_file(), manager + " receives native copy drag")
                assert (folder / outgoing).read_bytes() == (left / outgoing).read_bytes()
                print(manager+" outgoing native drag verified",flush=True)
                time.sleep(2)
                source = wait(lambda: named(incoming), incoming)
                target = wait(lambda: named("Right files"), "incoming file list")
                capture("before-incoming.png")
                source_point=center(source,manager_window,gtk=manager=="nautilus")
                if manager=="dolphin":
                    # A minimal KDE install may have no icon artwork. Hit the
                    # label portion of the accessible file cell, not its empty icon area.
                    r=source.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    source_point=(r.x+r.width//2,r.y+3*r.height//4)
                drag(source_point, center(target))
                time.sleep(1)
                capture("after-incoming.png")
                print("windows after incoming:",run("wmctrl","-lp"),flush=True)
                def copy_button():
                    for frame in walk(pyatspi.Registry.getDesktop(0)):
                        if frame.name == "Transfer dropped files":
                            for node in walk(frame):
                                if node.name=="Copy" and node.getRoleName()=="push button":return node
                confirm = wait(copy_button, "review incoming copy")
                assert not (right / incoming).exists(), "Incoming native drop must wait for confirmation"
                assert confirm.queryAction().doAction(0), "Native Copy button must expose its accessible action"
                wait(lambda: (right / incoming).is_file(), "confirmed incoming copy")
                assert (right / incoming).read_bytes() == (folder / incoming).read_bytes()
                assert (left / outgoing).exists() and (folder / incoming).exists()
                evidence.append(manager + ": real copy drag both directions; incoming review and byte equality passed")
                process.terminate()
                process.wait(timeout=10)
                run("wmctrl", "-ia", app_window)
            # Exercise real key focus and persist the accessible names/roles/states.
            click(wait(lambda: named("nautilus-outgoing.txt"), "focus file row"))
            run("xdotool", "key", "space", "F10", "Right", "Down", "Escape", "Escape")
            time.sleep(1)
            tree = []
            for node in walk(pyatspi.Registry.getDesktop(0)):
                try:
                    if node.name and ("files" in node.name.lower() or ".txt" in node.name):
                        tree.append({"name": node.name, "role": node.getRoleName(), "description":node.description, "selected": node.getState().contains(pyatspi.STATE_SELECTED)})
                except (LookupError, RuntimeError):
                    pass
            assert any(n["name"]=="nautilus-outgoing.txt" and n["selected"] and "marked" in n["description"] for n in tree),"Marked row must expose its persistent selection separately from cursor focus"
            speech=(args.output/"orca-debug.log").read_text()
            assert "SPEECH OUTPUT:" in speech and "outgoing.txt" in speech and "Left files" in speech,"Orca must handle real file-list and filename events"
            (args.output / "accessible-files.json").write_text(json.dumps(tree, indent=2))
            import gi
            gi.require_version("Gdk", "3.0")
            from gi.repository import Gdk
            screen = Gdk.get_default_root_window()
            Gdk.pixbuf_get_from_window(screen, 0, 0, screen.get_width(), screen.get_height()).savev(str(args.output / "desktop.png"), "png", [], [])
            evidence.append("AT-SPI file lists/rows, mark selection and real keyboard focus passed; Orca event/speech generation verified; high-contrast screenshot captured")
            (args.output / "results.txt").write_text("\n".join(evidence) + "\n")
            print("\n".join(evidence), flush=True)
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for log in logs:
            log.close()


if __name__ == "__main__":
    main()
