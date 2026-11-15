import sys
from rig import *
r = Rig("smoke", 9200)
try:
    print("host", r.get("/api/host"))
    print("hippo sockets", own_socket_paths(r.hippo.pid))
    print("libri sockets", own_socket_paths(r.daemon.pid))
    print(r.send("hello"))
    print(until(lambda: r.on_page("answered"), within=20), r.shown())
    time.sleep(1)
    print(r.chat())
    print(r.task())
    print(r.log())
finally:
    r.close()
