# Catch the #383 SIGABRT (swift "deallocated with non-zero retain count") and inspect the
# IAGSubgraphStorage being CFRelease'd by cf_ptr: its CFRuntimeBase bytes (refcount) + subgraph ptr.
set pagination off
set print elements 0
run
echo \n==== SIGABRT — #383 over-release ====\n
bt
python
import gdb, struct
# find the cf_ptr::~cf_ptr frame and read its _storage
f = gdb.newest_frame(); found = None
while f is not None:
    nm = f.name() or ""
    if "cf_ptr" in nm and "~cf_ptr" in nm:
        found = f; break
    f = f.older()
if found is None:
    # fall back: any frame mentioning cf_ptr
    f = gdb.newest_frame()
    while f is not None:
        if "cf_ptr" in (f.name() or ""): found = f; break
        f = f.older()
if found is not None:
    found.select()
    try:
        st = int(gdb.parse_and_eval("_storage"))
        print("[#383] cf_ptr _storage (IAGSubgraphStorage*) = 0x%x" % st)
        mem = gdb.selected_inferior().read_memory(st, 32).tobytes()
        print("[#383] storage first 32 bytes: " + " ".join("%02x" % b for b in mem))
        # IAGSubgraphStorage: CFRuntimeBase base; Subgraph* subgraph;  (subgraph ptr after the base)
        for off in (8, 12, 16, 20, 24):
            v = struct.unpack_from("<I", mem, off)[0]
            print("   [+%d] = %u (0x%x)" % (off, v, v))
    except Exception as e:
        print("[#383] read _storage failed:", e)
else:
    print("[#383] cf_ptr frame not found")
end
quit
