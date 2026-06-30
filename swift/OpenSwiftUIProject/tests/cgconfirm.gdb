# Confirm the parent-subgraph UAF: break on Subgraph::add_child, find the call whose _children
# push_back targets the frame-15 cg slot, and dump the parent subgraph's state.
set pagination off
set print elements 0
python
import gdb, re, struct
FRAME15_OFF = 49589328
st = {'base': None, 'sz': None}

def mappings():
    out = gdb.execute("info proc mappings", to_string=True); res = []
    for line in out.splitlines():
        m = re.match(r'\s*0x([0-9a-fA-F]+)\s+0x([0-9a-fA-F]+)', line)
        if m: res.append((int(m.group(1),16), int(m.group(2),16)))
    return res

def find_base():
    inf = gdb.selected_inferior(); pat = struct.pack("<Q", 0xC0FFEE99DEADBEEF)
    for (s, e) in mappings():
        if e - s > 512*1024*1024: continue
        try: blob = inf.read_memory(s, e-s).tobytes()
        except Exception: continue
        i = blob.find(pat)
        if i != -1: return s + i - 39507864
    return None

class AddChildBP(gdb.Breakpoint):
    def __init__(self): super().__init__("IAG::Subgraph::add_child")
    def stop(self):
        try:
            buf = int(gdb.parse_and_eval("__this->_children._buffer"))
            size = int(gdb.parse_and_eval("(int)__this->_children._size"))
        except Exception:
            return False
        if buf == 0: return False
        if st['sz'] is None:
            try: st['sz'] = int(gdb.parse_and_eval("sizeof(IAG::Subgraph::SubgraphChild)"))
            except Exception: st['sz'] = 8
        if st['base'] is None:
            st['base'] = find_base()
            if st['base'] is None: return False
        target = buf + size * st['sz']
        cg = st['base'] + FRAME15_OFF
        if not (cg - 32 <= target <= cg + 96):
            return False
        print("\n[CONFIRM] add_child writing into cg slot region!")
        print("  base=0x%x cg=0x%x target=0x%x (buf=0x%x size=%d sz=%d)" % (st['base'], cg, target, buf, size, st['sz']))
        try:
            print("  __this->_object       = %s  (null => clear_object ran => subgraph DEAD)" % gdb.parse_and_eval("__this->_object"))
        except Exception as e: print("  _object: ", e)
        for f in ("_invalidation_state", "_flags", "_context_id", "_children._capacity"):
            try: print("  __this->%s = %s" % (f, gdb.parse_and_eval("__this->%s" % f)))
            except Exception as e: print("  %s: %s" % (f, e))
        try: print("  graph registered? %s" % gdb.parse_and_eval("__this->_graph->contains_subgraph(__this)"))
        except Exception as e: print("  contains_subgraph: ", e)
        return True   # stop here
AddChildBP()
end
run
echo \n==== CONFIRM done ====\n
bt
quit
