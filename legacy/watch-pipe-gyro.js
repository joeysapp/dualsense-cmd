let fs = require('node:fs');
let DSPipePath = './ds_pipe';
let pipe = fs.createReadStream(DSPipePath, { flags: fs.constants.O_RDWR });

// Serialization/optimizations for JSON: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON/parse
let x, y, z;
pipe.on('data', (data) => {
  let msg = JSON.parse(data);

  console.clear();
  x = ("" + msg.gyroscope.pos[0]).padStart(5, ' ')
  y = ("" + msg.gyroscope.pos[1]).padStart(5, ' ')
  z = ("" + msg.gyroscope.pos[2]).padStart(5, ' ')

  console.log(`gyroscope\n  x ${x}\n  y ${y}\n  z ${z}`);
});
