import threading, urllib.request, time
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
seen, lock = [], threading.Lock()
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"])).decode()
        with lock: seen.append(body)
        self.send_response(200); self.end_headers(); self.wfile.write(b"ok")
    def log_message(self, *a): pass
srv = ThreadingHTTPServer(("127.0.0.1", 0), H); port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()
def worker(i):
    for k in range(5):
        urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{port}/", data=f"{i}.{k}".encode(), method="POST")).read()
ts = [threading.Thread(target=worker, args=(i,)) for i in range(6)]
for t in ts: t.start()
for t in ts: t.join()
srv.shutdown()
print("n=%d" % len(seen)); print("order=" + ",".join(seen)); print("DONE")
