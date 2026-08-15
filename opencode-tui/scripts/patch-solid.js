// Patch @opentui/solid to use solid-js instead of solid-js/dist/solid.js
const fs = require('fs')
const path = require('path')
const file = path.join(__dirname, '../node_modules/@opentui/solid/index.js')
if (fs.existsSync(file)) {
  let content = fs.readFileSync(file, 'utf8')
  const before = content
  content = content.replace(/from "solid-js\/dist\/solid\.js"/g, 'from "solid-js"')
  if (content !== before) {
    fs.writeFileSync(file, content)
    console.log('Patched @opentui/solid/index.js')
  }
}
