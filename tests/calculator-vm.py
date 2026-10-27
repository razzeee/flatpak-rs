"""Opt-in acceptance of the exact unmodified, signed Calculator payloads."""
import json
import shlex
import datetime

machine.start()
machine.wait_for_unit("user@1000.service")
machine.wait_for_file("/run/user/1000/wayland-1")

def alice(command):
    return "su - alice -c " + shlex.quote("export XDG_RUNTIME_DIR=/run/user/1000; " + command)

machine.succeed(alice("systemd-run --user --unit=calculator-repo --property=StandardError=null python -m http.server 8080 --bind 127.0.0.1 --directory /etc/calculator-repo"))
machine.wait_for_open_port(8080)
remote = json.loads(machine.succeed(alice("flatpak-rs remote-add flathub org.flathub.Stable /etc/calculator.flatpakrepo")))
plan = json.loads(machine.succeed("cat /etc/calculator-plan.json"))
for key in ["app_provider", "runtime_provider"]:
    plan[key] = {"name": "flathub", "generation": remote["generation"]}
machine.succeed("printf %s " + shlex.quote(json.dumps(plan)) + " > /home/alice/calculator-plan.json")
installed = json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/calculator-plan.json")))
assert installed["installed"]["app"] == plan["app"]
assert installed["installed"]["runtime"] == plan["runtime"]
machine.succeed(alice("flatpak-rs remote-disable flathub"))
machine.succeed(alice("systemctl --user stop calculator-repo"))
assert json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/calculator-plan.json")))["status"] == "unchanged"
machine.succeed(alice("systemd-run --user --unit=calculator-client sh -c 'flatpak-rs run org.flathub.Stable org.gnome.Calculator x86_64 stable --wayland wayland-1 > /home/alice/calculator.out 2>/home/alice/calculator.err'"))
try:
    machine.wait_until_succeeds(alice("swaymsg -s /run/user/1000/sway.sock -t get_tree | grep gnome-calculator"), timeout=30)
    machine.succeed(alice("swaymsg -s /run/user/1000/sway.sock '[app_id=gnome-calculator] floating enable, resize set 420 600, move position 20 20, focus'"))
    machine.succeed(alice("swaymsg -s /run/user/1000/sway.sock seat seat0 cursor set 200 150"))
    machine.succeed(alice("swaymsg -s /run/user/1000/sway.sock seat seat0 cursor press button1"))
    machine.succeed(alice("swaymsg -s /run/user/1000/sway.sock seat seat0 cursor release button1"))
    machine.send_key("2")
    machine.send_key("shift-equal")
    machine.send_key("3")
    machine.send_key("ret")
    # The display region excludes the keypad: a '5' button is not evidence of
    # the calculation result. Keep both pixels and OCR text for inspection.
    def result_visible(_last):
        machine.succeed(alice("WAYLAND_DISPLAY=wayland-1 grim /home/alice/calculator.png"))
        machine.succeed("python -c \"from PIL import Image; Image.open('/home/alice/calculator.png').crop((390,274,438,308)).resize((192,136)).save('/home/alice/calculator-result.png')\"")
        code, text = machine.execute("tesseract /home/alice/calculator-result.png stdout --psm 7 2>/dev/null")
        return code == 0 and "5" in [line.strip() for line in text.splitlines()]
    retry(result_visible, timeout=datetime.timedelta(seconds=30))
    machine.screenshot("calculator-two-plus-three")
    for name in ["calculator.png", "calculator-result.png", "calculator-plan.json", "calculator.err"]:
        machine.copy_from_machine("/home/alice/" + name)
finally:
    machine.screenshot("calculator-final-state")
    print(machine.succeed("cat /home/alice/calculator.err"))
    machine.succeed(alice("systemctl --user stop calculator-client"))
machine.wait_until_succeeds(alice("flatpak-rs instances | grep -F '\"instances\": []'"))
