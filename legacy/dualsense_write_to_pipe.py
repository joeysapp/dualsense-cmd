# [resources]
# https://github.com/flok/pydualsense
# For this to work on M1 macs, you need to install a different hidapi from here:
#   https://github.com/trezor/cython-hidapi
#   (which itself uses a different hidapi)
#
# There's some other options here, but they didn't work for me:
#   https://stackoverflow.com/questions/59188993/osx-hidapi-install-for-python3-pip3-troubleshooting-high-sierra

# [setup]
# To connect a dualsense controller on macos, hold the create button and power button.

# [todo]
# Necessaray to prevent overflows if a ctrl-c is called?

import pydualsense.pydualsense as pydualsense
import time, math, os, json, random

ds = pydualsense()
ds.init()

i = 0
iters = 10000
dt = 0.02;
while i < iters:
    with open('./ds_pipe', 'w') as ds_pipe:
        # Write to a pipe for other processes to tail
        ds_pipe.write(f'{ds.to_object()}')
    if i % 7 == 0:
        ds.set_led_touchpad_color(255*random.random(), 255*random.random(), 255*random.random())
        ds.set_lrumble((i/50)*255)
        ds.set_rrumble((i/50)*255)
    time.sleep(dt)
    i += 1

ds.set_led_touchpad_color(255, 255, 255);
ds.set_lrumble(0);
ds.set_rrumble(0);
ds.close()
