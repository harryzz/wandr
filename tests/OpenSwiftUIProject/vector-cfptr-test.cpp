// [#383] Isolated reproduction: IAG::vector<cf_ptr, N> realloc vs cf_ptr's non-trivial dtor.
// Faithful copy of Vector/Vector.h's realloc_vector + the general inline-capacity vector template,
// driven exactly like std::stack in Subgraph::update, with a cf_ptr-shaped refcount probe.
// If the realloc relocation mishandles cf_ptr, a storage's refcount ends != 0 (over-release/leak).
// Build: clang++ -std=c++20 -fsanitize=address,undefined -g vector-cfptr-test.cpp -o /tmp/vt && /tmp/vt
#include <bit>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <stack>
#include <type_traits>
#include <utility>

static size_t malloc_good_size(size_t n) { return (n + 15) & ~size_t(15); } // typical 16-byte rounding
[[noreturn]] static void precondition_failure(const char *m) { fprintf(stderr, "PRECONDITION: %s\n", m); abort(); }

// ---- refcount bookkeeping (stands in for CFRetain/CFRelease on IAGSubgraphStorage) ----
static std::map<void *, long> g_rc;   // storage -> live refcount
static long g_min_rc = 0;             // lowest refcount any storage reached (negative => over-release)
static void RC_retain(void *p) { if (p) g_rc[p]++; }
static const char *g_phase = "?";
static void RC_release(void *p) {
    if (p) { long v = --g_rc[p]; if (v < g_min_rc) g_min_rc = v;
             if (v < 0) printf("  !! OVER-RELEASE storage=%p -> %ld  during [%s]\n", p, v, g_phase); }
}

// ---- cf_ptr clone (exact retain/release shape of util::cf_ptr) ----
struct cf_ptr {
    void *_storage;
    cf_ptr() noexcept : _storage(nullptr) {}
    explicit cf_ptr(void *r) : _storage(r) { if (_storage) RC_retain(_storage); }
    ~cf_ptr() { if (_storage) { RC_release(_storage); _storage = nullptr; } }
    cf_ptr(const cf_ptr &o) noexcept : _storage(o._storage) { if (_storage) RC_retain(_storage); }
    cf_ptr(cf_ptr &&o) noexcept : _storage(std::exchange(o._storage, nullptr)) {}
    cf_ptr &operator=(const cf_ptr &o) noexcept {
        if (this != &o) { void *t = o._storage; if (t) RC_retain(t); if (_storage) RC_release(_storage); _storage = t; }
        return *this;
    }
    cf_ptr &operator=(cf_ptr &&o) noexcept {
        if (this != &o) { if (_storage) RC_release(_storage); _storage = std::exchange(o._storage, nullptr); }
        return *this;
    }
    void *get() const noexcept { return _storage; }
};

// ---- realloc_vector: EXACT copy of Vector/Vector.h:117-150 ----
namespace details {
template <typename size_type, unsigned int element_size_bytes>
    requires std::unsigned_integral<size_type>
void *realloc_vector(void *buffer, void *_inline_buffer, size_type inline_capacity, size_type *size,
                     size_type preferred_new_size) {
    if (preferred_new_size <= inline_capacity) {
        if (buffer) {
            memcpy(_inline_buffer, buffer, preferred_new_size * element_size_bytes);
            free(buffer);
            *size = inline_capacity;
        }
        return nullptr;
    }
    size_t new_size_bytes = malloc_good_size(preferred_new_size * element_size_bytes);
    size_type new_size = (size_type)(new_size_bytes / element_size_bytes);
    if (new_size == *size) {
        return buffer;
    }
    void *new_buffer = realloc(buffer, new_size_bytes);
    if (!new_buffer) precondition_failure("allocation failure");
    if (!buffer) {
        memcpy(new_buffer, _inline_buffer, (*size) * element_size_bytes);
        memset(_inline_buffer, 0, (*size) * element_size_bytes); // [#383 FIX] elements relocated to heap;
        // clear the inline source so the compiler's automatic destruction of the _inline_buffer[] member
        // doesn't re-run T's destructor on the now-moved-from slots (double-release for cf_ptr).
    }
    *size = new_size;
    return new_buffer;
}
} // namespace details

// ---- vector<T, inline_cap, size_type>: EXACT copy of the general template ----
template <typename T, unsigned int _inline_capacity = 0, typename _size_type = std::size_t>
    requires std::unsigned_integral<_size_type>
class vector {
  public:
    using value_type = T;
    using reference = T &;
    using const_reference = const T &;
    using size_type = _size_type;
  private:
    T _inline_buffer[_inline_capacity];
    T *_buffer = nullptr;
    size_type _size = 0;
    size_type _capacity = _inline_capacity;
    void reserve_slow(size_type new_cap) {
        size_type effective_new_cap = std::max(capacity() * 1.5, new_cap * 1.0);
        _buffer = reinterpret_cast<T *>(details::realloc_vector<size_type, sizeof(T)>(
            (void *)_buffer, (void *)_inline_buffer, _inline_capacity, &_capacity, effective_new_cap));
    }
  public:
    vector() {}
    ~vector() {
        for (size_type i = 0; i < _size; i++) data()[i].~T();
        if (_buffer) free((void *)_buffer);
    }
    vector(const vector &) = delete;
    vector &operator=(const vector &) = delete;
    vector(vector &&other) noexcept
        : _size(std::exchange(other._size, 0)), _capacity(std::exchange(other._capacity, _inline_capacity)) {
        if (other._buffer) { _buffer = std::exchange(other._buffer, nullptr); }
        else {
            for (size_type i = 0; i < _size; ++i) { new (&_inline_buffer[i]) T(std::move(other._inline_buffer[i])); other._inline_buffer[i].~T(); }
            _buffer = nullptr;
        }
    }
    vector &operator=(vector &&) noexcept = delete;
    T &operator[](size_type p) { return data()[p]; }
    T &back() { return data()[_size - 1]; }
    T *data() { return _buffer != nullptr ? _buffer : _inline_buffer; }
    bool empty() const { return _size == 0; }
    size_type size() const { return _size; }
    void reserve(size_type new_cap) { if (new_cap <= capacity()) return; reserve_slow(new_cap); }
    size_type capacity() const { return _capacity; }
    void push_back(const T &value) { reserve(_size + 1); new (&data()[_size]) T(value); _size += 1; }
    void push_back(T &&value) { reserve(_size + 1); new (&data()[_size]) T(std::move(value)); _size += 1; }
    void pop_back() { data()[_size - 1].~T(); _size -= 1; }
};

// ---- driver: mimic Subgraph::update's stack walk (copy top, pop, push children, repeat) ----
static long g_next = 1;
static void *make_storage() { return (void *)(uintptr_t)(g_next++ * 16); }

int main() {
    // Build a tree and DFS it through the cf_ptr stack, exactly like Subgraph::update.
    // INLINE_CAP small (4) so pushing children forces realloc — the suspected trigger.
    using Stack = std::stack<cf_ptr, vector<cf_ptr, 4, std::uint64_t>>;
    int created = 0;
    {
        Stack stk;
        // each node pushes `fanout` children when processed, up to a budget — drives realloc churn.
        int budget = 5000; // small: keep the trace readable
        void *root = make_storage(); created++;
        g_phase = "push-root"; stk.push(cf_ptr(root));
        while (!stk.empty()) {
            g_phase = "copy-top"; cf_ptr obj = stk.top();
            g_phase = "pop"; stk.pop();
            if (budget > 0) {
                int fanout = 3;
                for (int i = 0; i < fanout && budget > 0; i++) {
                    void *child = make_storage(); created++; budget--;
                    g_phase = "push-child"; stk.push(cf_ptr(child));
                }
            }
            g_phase = "obj-dtor";
        } // obj dtor each iteration
        g_phase = "stack-dtor";
    } // stack dtor
    g_phase = "after";

    // After everything is destroyed, every storage's refcount MUST be 0, and none ever went negative.
    long leaked = 0, overreleased = 0;
    for (auto &kv : g_rc) { if (kv.second > 0) leaked++; if (kv.second < 0) overreleased++; }
    printf("[#383 test] created=%d  min_refcount_ever=%ld  leaked(>0)=%ld  overreleased(<0)=%ld\n",
           created, g_min_rc, leaked, overreleased);
    if (g_min_rc < 0 || overreleased > 0) { printf("[#383 test] *** OVER-RELEASE REPRODUCED ***\n"); return 2; }
    if (leaked > 0) { printf("[#383 test] *** LEAK (under-release) ***\n"); return 3; }
    printf("[#383 test] BALANCED — no over-release, no leak\n");
    return 0;
}
