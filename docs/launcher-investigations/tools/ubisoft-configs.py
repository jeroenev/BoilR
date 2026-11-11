"""Lists the start_game blocks of Ubisoft Connect game configurations.
Input: the plaintext written by ubisoft-vdat-decrypt.ps1 -Out <file>.
Usage: ubisoft-configs.py <plaintext> [<id>,<id>...] [<regex over the YAML>]"""
import sys, re
sys.stdout.reconfigure(encoding='utf-8')
def varint(b, p):
    v = s = 0
    while True:
        x = b[p]; p += 1; v |= (x & 0x7f) << s
        if not x & 0x80: return v, p
        s += 7
def fields(b):
    p = 0; out = []
    while p < len(b):
        t, p = varint(b, p); f, w = t >> 3, t & 7
        if w == 0: v, p = varint(b, p)
        elif w == 2:
            n, p = varint(b, p); v = b[p:p+n]; p += n
        else: raise Exception('wt %d' % w)
        out.append((f, v))
    return out
d = open(sys.argv[1], 'rb').read()
recs = {}
for f, v in fields(d):
    if f != 1: continue
    sub = dict()
    for g, x in fields(v): sub.setdefault(g, x)
    recs[sub.get(1)] = sub
print('records', len(recs), 'fields', sorted({k for r in recs.values() for k in r}))
want = [int(x) for x in sys.argv[2].split(',')] if len(sys.argv) > 2 else []
for gid, r in sorted(recs.items()):
    y = r.get(3, b'').decode('utf-8', 'replace')
    m = re.search(r'^\s+name: (.*)$', y, re.M)
    dn = re.search(r'display_name: (.*)', y)
    if gid in want or (len(sys.argv) > 3 and re.search(sys.argv[3], y, re.I)):
        print('=' * 20, gid, r.get(2), m and m.group(1), dn and dn.group(1))
        s = y.find('start_game:'); e = y.find('\n  ', y.find('\n  ', s + 1) + 1)
        # print the start_game block (lines indented deeper than 2)
        lines = y[s:].split('\n'); blk = [lines[0]]
        for ln in lines[1:]:
            if ln.startswith('    ') or not ln.strip(): blk.append(ln)
            else: break
        print('\n'.join(blk))
        for k in ('third_party_platform', 'is_ulc', 'installer:', 'game_identifier', 'register:'):
            pass
