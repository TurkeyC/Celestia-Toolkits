# SSH to Linux from Mac, Windows

## In Linux

Install and configure PulseAudio:

    sudo apt install alsa-utils pulseaudio libasound2-plugins pulseaudio-utils
    mkdir -p ~/.config/pulse/
    vi ~/.config/pulse/default.pa  # or use nano, pico

Enter:
```
.fail
load-module module-native-protocol-tcp listen=127.0.0.1 auth-anonymous=1
.nofail
```

Restart PulseAudio services:

    systemctl --user start pipewire pipewire-pulse
    systemctl --user start pulseaudio.service pulseaudio.socket
    # check status
    systemctl --user status pulseaudio

Edit Alsa config:

    vi ~/.asoundrc

Enter:
```conf
pcm.!default {
    type pulse
}

ctl.!default {
    type pulse
}
```



## On Mac

Install PulseAudio:

    brew install pulseaudio

Start audio daemon:

    pulseaudio --load="module-native-protocol-tcp listen=127.0.0.1 auth-anonymous=1" --resample-method=speex-float-3 --exit-idle-time=-1 --daemon

Ignore the 'capabilities' warning.

Verify PulseAudio:

    pactl info  # shoud have default sink
    pactl list short sinks  # should list devices

Connect to remove Linux via SSH:

    ssh -o ExitOnForwardFailure=yes -R 127.0.0.1:24713:127.0.0.1:4713 user@ip-or-hostname

Now inside SSH linux:

    export PULSE_SERVER=127.0.0.1:24713

