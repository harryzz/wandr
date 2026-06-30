# The crash uses a GARBAGE CGContext (canvas handle 1668183366 = "Func"), distinct from the valid
# frame-15 cg. Break at process exit (linear memory still intact) and scan for it: any CGContext
# (metadata word == 38014828) whose canvas field (+8) == 1668183366, plus where the "Func" bytes live.
set pagination off
set print elements 0
python
import gdb, re, struct
CG_META = 38014828
BAD = 1668183366  # "Func"
st = {}

def mappings():
    out = gdb.execute("info proc mappings", to_string=True); res = []
    for line in out.splitlines():
        m = re.match(r'\s*0x([0-9a-fA-F]+)\s+0x([0-9a-fA-F]+)', line)
        if m: res.append((int(m.group(1),16), int(m.group(2),16)))
    return res

def wasm_region():
    inf = gdb.selected_inferior(); pat = struct.pack("<Q", 0xC0FFEE99DEADBEEF)
    for (s, e) in mappings():
        if e - s > 512*1024*1024: continue
        try: blob = inf.read_memory(s, e-s).tobytes()
        except Exception: continue
        if pat in blob:
            return s, blob
    return None, None

class ExitBP(gdb.Breakpoint):
    def __init__(self): super().__init__("exit", internal=True)
    def stop(self):
        base, blob = wasm_region()
        if base is None:
            print("[SCAN] wasm region not found (sentinel gone)"); return True
        print("[SCAN] wasm base=0x%x size=%dMB" % (base, len(blob)//1048576))
        # 1) all CGContext objects (metadata word == CG_META) and their canvas handle
        meta_b = struct.pack("<i", CG_META)
        i = 0; n = 0
        print("[SCAN] CGContext objects (off : canvas[+8] graphics[+12] shadow[+?]):")
        while True:
            i = blob.find(meta_b, i)
            if i < 0: break
            canvas = struct.unpack_from("<i", blob, i+8)[0]
            gfx = struct.unpack_from("<i", blob, i+12)[0]
            mark = "  <<< GARBAGE canvas=Func" if canvas == BAD else ""
            if 0 <= canvas <= 8 or canvas == BAD:  # only plausible cg (small handle) or the bad one
                print("   off=%d canvas=%d gfx=%d%s" % (i, canvas, gfx, mark)); n += 1
            i += 4
        print("[SCAN] (%d CGContext-like objects)" % n)
        # 2) every occurrence of the BAD handle value
        bad_b = struct.pack("<i", BAD); j = 0; cnt = 0
        print("[SCAN] occurrences of 1668183366 ('Func'):")
        while cnt < 30:
            j = blob.find(bad_b, j)
            if j < 0: break
            ctx = blob[max(0,j-8):j+16]
            print("   off=%d  ctx=%s" % (j, ctx.hex()))
            j += 4; cnt += 1
        return True
ExitBP()
end
run
quit
