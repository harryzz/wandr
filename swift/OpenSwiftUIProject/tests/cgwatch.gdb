# Definitive: watch cg.canvas (frame-15 slot) and LOG every write (prev->new + caller) without
# stopping. At exit, print the last writes — the genuine frame-15 corruption is the last
# handle->garbage transition before the canvas_save crash. No filter to defeat.
set pagination off
set print elements 0
python
import gdb, re, struct
FRAME15_OFF = 49589328
st = {'base': None, 'sent_off': None, 'armed': False, 'f15': False, 'writes': []}

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

def callers():
    out = []
    f = gdb.newest_frame(); n = 0
    while f is not None and n < 6:
        nm = f.name() or "?"
        if nm not in ("write","writev") and "memmove" not in nm and "memcpy" not in nm:
            out.append(nm[:60]); n += 1
        f = f.older()
    return out

class PrevWP(gdb.Breakpoint):
    def __init__(self, caddr, cgaddr):
        super().__init__("*(int*)0x%x" % caddr, gdb.BP_WATCHPOINT, gdb.WP_WRITE)
        self.caddr = caddr; self.cgaddr = cgaddr
        try: self.prev = struct.unpack("<i", gdb.selected_inferior().read_memory(caddr,4).tobytes())[0]
        except Exception: self.prev = 0
    def stop(self):
        inf = gdb.selected_inferior()
        try:
            new = struct.unpack("<i", inf.read_memory(self.caddr,4).tobytes())[0]
            meta = struct.unpack("<I", inf.read_memory(self.cgaddr,4).tobytes())[0]
        except Exception: return False
        prev = self.prev; self.prev = new
        rec = (st['f15'], prev, new, meta, callers()[:4])
        st['writes'].append(rec)
        if len(st['writes']) > 60: st['writes'].pop(0)
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
        if b"CGOBJ frame=15 " in s: st['f15'] = True
        if st['armed'] or b"CGOBJ frame=1 " not in s or st['sent_off'] is None: return False
        m = re.search(rb"off=(\d+) h=(-?\d+)", s); off, h = int(m.group(1)), int(m.group(2))
        ns = find_sentinel()
        if ns is None: return False
        st['base'] = ns - st['sent_off']
        mem = gdb.selected_inferior().read_memory(st['base']+off, 64).tobytes()
        ci = None
        for i in range(4,56,4):
            if struct.unpack_from("<i",mem,i)[0]==h and 0<struct.unpack_from("<i",mem,i+4)[0]<64: ci=i; break
        if ci is None: return False
        caddr = st['base'] + FRAME15_OFF + ci
        print("[CGWATCH] base=0x%x canvas@+%d -> log writes to 0x%x" % (st['base'], ci, caddr))
        PrevWP(caddr, st['base'] + FRAME15_OFF)
        st['armed'] = True
        return False
WBP()
end
run
echo \n==== last writes to cg.canvas (f15,prev,new,cg_meta,callers) ====\n
python
for (f15,prev,new,meta,cs) in st['writes'][-40:]:
    print("f15=%d %6d -> %-12d meta=%-10d | %s" % (f15, prev, new, meta, " <- ".join(cs)))
print("cg_meta should be 38014828 when the cg slot holds a live CGContext")
end
quit
