// This would probably be.... an endpoint? for the site/server?
// It would only watch the ReadStream if uh, something. 
let fs = require('node:fs');
let DSPipePath = '../ds_pipe';

// Making our stream as R+W means our node process is always accessing it - no closure by OS.
// [resource] https://stackoverflow.com/a/63803292
let pipe = fs.createReadStream(DSPipePath, { flags: fs.constants.O_RDWR });

pipe.on('data', (data) => {
  // [todo] Serialization/optimizations for JSON (if JSON over blobs)
  // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON/parse
  let msg = JSON.parse(data);
  console.clear();

  // console.log(`time: ${msg.time}`);

  // [note] This is just for pretty-printing, essentially
  console.dir({
    l_joystick: msg.lj,
    l_joystick_down: msg.lj_pressed,

    l_bumper: msg.l1,
    l_trigger: msg.l2,
    l_trigger_down: msg.l2_pressed,

    l_dpad: [msg.dpad_left, msg.dpad_up, msg.dpad_right, msg.dpad_down],
  }, {colors: true});

  console.dir({
    r_joystick: msg.rj,
    r_joystick_down: msg.rj_pressed,

    r_bumper: msg.r1,
    r_trigger: msg.r2,
    r_trigger_down: msg.r2_pressed,

    square: msg.ps_square,
    triangle: msg.ps_triangle,
    circle: msg.ps_circle,
    cross: msg.ps_cross,
  });
  // console.dir({ left_rumble: msg.lrumble, right_rumble: msg.rrumble });

  // console.dir({
  //   options_button: msg.btn_opts,
  //   share_button: msg.btn_share,
  //   mic_button: msg.btn_mic,
  //   playstation_button: msg.btn_ps,    
  // })
  // console.log('\ntouchpad is still weird')
  // console.dir({ left_touchpad: msg.ltouchpad.xy, right_touchpad: msg.rtouchpad.xy })
});
