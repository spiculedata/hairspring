"""The fixed consumer: deduplication is a first-write-wins marker in the Dapr state store,
which survives restarts. Everything else is identical to buggy_app.py."""
import buggy_app as base


def first_time(order_id):
    status, _ = base.call("POST", base.SIDECAR, [{"key": f"seen-{order_id}", "value": 1, "options": {"concurrency": "first-write"}}])
    return status == 204


base.first_time = first_time

if __name__ == "__main__":
    base.ThreadingHTTPServer(("127.0.0.1", int(base.os.environ["APP_PORT"])), base.Handler).serve_forever()
