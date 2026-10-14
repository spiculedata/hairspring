import os, time
r, w = os.pipe(); kids = []
for i in range(4):
    pid = os.fork()
    if pid == 0:
        time.sleep(int.from_bytes(os.urandom(1), "big") / 255 * 0.02)
        os.write(w, f"{i}\n".encode()); os._exit(0)
    kids.append(pid)
os.close(w); data = b""
while True:
    chunk = os.read(r, 64)
    if not chunk: break
    data += chunk
done = [os.waitpid(k, 0)[0] for k in kids]
print("order=" + ",".join(data.decode().split())); print("kids_reaped=%d" % len(done)); print("DONE")
