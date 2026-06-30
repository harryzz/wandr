# Watch the CGSink.cg field itself (not cg.canvas). Find sink's address (SINK log) + the cg-pointer
# field offset (the word == frame-1 cg), then hardware-watch it. Stop when it's written a non-heap
# garbage pointer (small, non-zero, like 63064) — the backtrace names what corrupts sink.cg.
set pagination off
set print elements 0
python
import gdb, re, struct
st = {'base': None, 'sent_off': None, 'sink_off': None, 'armed': False}

def mappings():
    out = gdb.execute("info proc mappings", to_string=True); res = []
    for line in out.splitlines():
        m = re.match(r'\s*0x([0-9a-fA-F]+)\s+0x([0-9a-fA-F]+)', line)
        if m: res.append((int(m.group(1),16), int(m.group(2),16)))
    return res

def find_sentinel():
    inf = gdb.selected_inferior(); pat = struct.pack("<Q", 0xC0FFEE99DEADBEEF)
    for (s, e) in mappings():
        if e - s > 512*1024*1024: continue
        try: blob = inf.read_memory(s, e-s).tobytes()
        except Exception: continue
        i = blob.find(pat)
        if i != -1: return s + i
    return None

class SinkWP(gdb.Breakpoint):
    def __init__(self, addr):
        super().__init__("*(int*)0x%x" % addr, gdb.BP_WATCHPOINT, gdb.WP_WRITE)
        self.addr = addr
    def stop(self):
        try: v = struct.unpack("<I", gdb.selected_inferior().read_memory(self.addr,4).tobytes())[0]
        except Exception: return False
        # legit: 0 (nil) or a real heap pointer (>=1MB). garbage: small non-zero (e.g. 63064).
        if 0 < v < 0x100000:
            print("[SINKWATCH] *** sink.cg written GARBAGE = %d (0x%x) ***" % (v, v))
            return True
        return False

class WBP(gdb.Breakpoint):
    def __init__(self): super().__init__("write", internal=True)
    def stop(self):
        try:
            buf = int(gdb.parse_and_eval("$rsi")); cnt = int(gdb.parse_and_eval("$rdx"))
            if not (0 < cnt <= 65536): return False
            s = gdb.selected_inferior().read_memory(buf, min(cnt,512)).tobytes()
        except Exception: return False
        if b"SENTINEL off=" in s and st['sent_off'] is None:
            st['sent_off'] = int(re.search(rb"SENTINEL off=(\d+)", s).group(1))
        if b"SINK off=" in s and st['sink_off'] is None:
            st['sink_off'] = int(re.search(rb"SINK off=(\d+)", s).group(1))
        if st['armed'] or b"CGOBJ frame=1 " not in s: return False
        if st['sent_off'] is None or st['sink_off'] is None: return False
        m = re.search(rb"off=(\d+) h=(-?\d+)", s); cg1 = int(m.group(1))
        ns = find_sentinel()
        if ns is None: return False
        st['base'] = ns - st['sent_off']
        sink = st['base'] + st['sink_off']
        mem = gdb.selected_inferior().read_memory(sink, 80).tobytes()
        # cg field = the word holding the frame-1 cg pointer (== cg1)
        coff = None
        for i in range(0, 80, 4):
            if struct.unpack_from("<I", mem, i)[0] == cg1: coff = i; break
        if coff is None:
            print("[SINKWATCH] cg field not found in sink; sink words: " + " ".join("%d:%d"%(i,struct.unpack_from("<i",mem,i)[0]) for i in range(0,80,4)))
            st['armed'] = True; return False
        addr = sink + coff
        print("[SINKWATCH] base=0x%x sink@%d cg_field@+%d (val=%d) -> watch 0x%x" % (st['base'], st['sink_off'], coff, cg1, addr))
        SinkWP(addr)
        st['armed'] = True
        return False
WBP()
end
run
echo \n==== sink.cg corrupted — backtrace (the writer) ====\n
bt
quit
