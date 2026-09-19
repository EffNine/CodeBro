import json, subprocess, os, sys
env = dict(os.environ)
for k, v in json.loads(sys.argv[1]).items(): env[k] = v
p = subprocess.Popen(["codebro", "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, cwd="/tmp/opencode/phase0/probe/work")
def send(o):
    p.stdin.write((json.dumps(o) + "\n").encode()); p.stdin.flush()
    return json.loads(p.stdout.readline().decode())
send({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}})
p.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); p.stdin.flush()
ms = send({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_stats","arguments":{}}})
print(json.dumps(ms)[:400]); p.kill()
