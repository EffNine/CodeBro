// Patch @opentui/solid to use solid-js instead of solid-js/dist/solid.js
// This ensures both the application and @opentui/solid share the same SolidJS runtime.
// Bun resolves "solid-js" to dist/server.js; without this patch, @opentui/solid
// imports from dist/solid.js, creating two separate runtime instances and breaking
// createContext/useContext propagation.
const fs = require('fs')
const path = require('path')

const solidPkgDir = path.join(__dirname, '../node_modules/@opentui/solid')
const files = ['index.js', 'index.bun.js']

for (const file of files) {
  const filePath = path.join(solidPkgDir, file)
  if (!fs.existsSync(filePath)) {
    console.log(`Skip ${file}: not found`)
    continue
  }
  let content = fs.readFileSync(filePath, 'utf8')
  const before = content
  content = content.replace(/from "solid-js\/dist\/solid\.js"/g, 'from "solid-js"')
  if (content !== before) {
    fs.writeFileSync(filePath, content)
    console.log(`Patched @opentui/solid/${file}`)
  } else {
    console.log(`No changes needed for @opentui/solid/${file}`)
  }
}
