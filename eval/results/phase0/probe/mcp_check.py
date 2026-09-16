import json, subprocess, sys, os

def rpc(proc, method, params=None, nid=1):
    msg = {"jsonrpc": "2.0", "id": nid, "method": method}
    if params is not None: msg["params"] = params
    proc.stdin.write((json.dumps(msg) + "\n").encode()); proc.stdin.flush()
    line = proc.stdout.readline().decode()
    return json.loads(line)

def main():
    extra = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}
    mode = extra.pop("mode", "env")
    env = dict(os.environ); env.update(extra)
    args = ["codebro", "serve"] + (["--root", "/home/afnan/projects/active/codebro"] if mode == "flag" else [])
    p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, cwd="/tmp/opencode/phase0/probe/work")
    init = rpc(p, "initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "probe", "version": "0"}})
    print("server:", init.get("result", {}).get("serverInfo", {}).get("name"), init.get("result", {}).get("serverInfo", {}).get("version"))
    p.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); p.stdin.flush()
    tools = rpc(p, "tools/list", {}, 2)["result"]["tools"]
    print("tool_count:", len(tools))
    ms = rpc(p, "tools/call", {"name": "codebro_memory_stats", "arguments": {}}, 3)
    txt = ms["result"]["content"][0].get("text", "")
    print("memory_stats:", txt[:200].replace("\n", " "))
    wc = rpc(p, "tools/call", {"name": "codebro_workspace_context", "arguments": {}}, 4)
    wtxt = wc["result"]["content"][0].get("text", "")
    print("workspace_root_seen:", "active/codebro" in wtxt)
    p.kill()

main()
