"""An order consumer with a crash-recovery bug.

It deduplicates redelivered messages with an in-memory set. That works until the process
restarts: the set is lost, the broker redelivers (at-least-once), and the order is applied twice.
Dapr's state store survives the restart; the app's memory does not.
"""
import json
import os
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SIDECAR = f"http://127.0.0.1:{os.environ['DAPR_HTTP_PORT']}/v1.0/state/statestore"
EXPECTED_IDS = [int(i) for i in os.environ.get("EXPECTED_IDS", "1,2,3").split(",")]
seen = set()           # BUG: lost on restart
lock = threading.Lock()


def call(method, url, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def effect_count(order_id):
    status, body = call("GET", f"{SIDECAR}/effect-{order_id}")
    return int(json.loads(body)) if status == 200 else 0


def first_time(order_id):
    """Returns True the first time this order is seen. Overridden by fixed_app.py."""
    with lock:
        if order_id in seen:
            return False
        seen.add(order_id)
        return True


def apply_order(order_id):
    call("POST", SIDECAR, [{"key": f"effect-{order_id}", "value": effect_count(order_id) + 1}])


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, payload):
        body = json.dumps(payload).encode() if not isinstance(payload, str) else payload.encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/dapr/subscribe":
            return self.reply(200, [{"pubsubname": "pubsub", "topic": "orders", "route": "/orders"}])
        if self.path == "/anthill/check":
            for order_id in EXPECTED_IDS:
                n = effect_count(order_id)
                if n != 1:
                    return self.reply(500, f"order {order_id} was applied {n} times")
            return self.reply(200, "ok")
        self.reply(404, "not found")

    def do_POST(self):
        if self.path != "/orders":
            return self.reply(404, "not found")
        event = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        order_id = event["data"]["id"]
        if first_time(order_id):
            apply_order(order_id)
        self.reply(200, {"status": "SUCCESS"})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(os.environ["APP_PORT"])), Handler).serve_forever()
