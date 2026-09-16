import json, subprocess, os
env = dict(os.environ); env["CODEBRO_WORKSPACE_ROOT"] = "/home/afnan/projects/active/codebro"
p = subprocess.Popen(["codebro", "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, cwd="/tmp/opencode/phase0/probe/work")
def send(o):
    p.stdin.write((json.dumps(o) + "\n").encode()); p.stdin.flush()
    return json.loads(p.stdout.readline().decode())
print(send({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}))
p.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); p.stdin.flush()
names = [t["name"] for t in send({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})["result"]["tools"]]
print("names_sample:", names[:8])
print("has_memory_stats:", "codebro_memory_stats" in names)
r = send({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"codebro_memory_stats","arguments":{}}})
print("call_response_keys:", list(r.keys()))
print(json.dumps(r)[:600])
p.kill()
