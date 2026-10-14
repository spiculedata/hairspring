import socket, threading, random, time
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0)); srv.listen(16); port = srv.getsockname()[1]
def client(i):
    time.sleep(random.random() * 0.02)
    c = socket.create_connection(("127.0.0.1", port)); c.sendall(f"{i}\n".encode()); c.close()
ts = [threading.Thread(target=client, args=(i,)) for i in range(8)]
for t in ts: t.start()
order = []
for _ in range(8):
    conn, _ = srv.accept(); order.append(conn.recv(16).decode().strip()); conn.close()
for t in ts: t.join()
print("order=" + ",".join(order)); print("DONE")
