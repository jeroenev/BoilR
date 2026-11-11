param([string]$Vdat = "$env:LOCALAPPDATA\Ubisoft Game Launcher\cache\configuration\configurations.vdat", [string]$Out)
# Decrypt a Ubisoft Connect .vdat cache file: protobuf {1: header {1:1, 2:{1:1, 2:keyId, 3:nonce, 4:tag}}, 2: ciphertext};
# key = DPAPI(CurrentUser)-protected blob in %LOCALAPPDATA%\Ubisoft Game Launcher\store\<keyId>.bin; AES-256-GCM.
Add-Type -AssemblyName System.Security
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class Gcm {
  [DllImport("bcrypt.dll")] static extern int BCryptOpenAlgorithmProvider(out IntPtr h, [MarshalAs(UnmanagedType.LPWStr)] string alg, IntPtr impl, int flags);
  [DllImport("bcrypt.dll")] static extern int BCryptSetProperty(IntPtr h, [MarshalAs(UnmanagedType.LPWStr)] string prop, byte[] val, int len, int flags);
  [DllImport("bcrypt.dll")] static extern int BCryptGenerateSymmetricKey(IntPtr alg, out IntPtr key, IntPtr obj, int objLen, byte[] secret, int len, int flags);
  [DllImport("bcrypt.dll")] static extern int BCryptDecrypt(IntPtr key, byte[] input, int inLen, ref INFO info, byte[] iv, int ivLen, byte[] output, int outLen, out int res, int flags);
  [StructLayout(LayoutKind.Sequential)] public struct INFO { public int cbSize; public int dwInfoVersion; public IntPtr pbNonce; public int cbNonce; public IntPtr pbAuthData; public int cbAuthData; public IntPtr pbTag; public int cbTag; public IntPtr pbMacContext; public int cbMacContext; public int cbAAD; public long cbData; public int dwFlags; }
  public static byte[] Decrypt(byte[] key, byte[] nonce, byte[] tag, byte[] ct) {
    IntPtr alg, k; int st;
    st = BCryptOpenAlgorithmProvider(out alg, "AES", IntPtr.Zero, 0); if (st != 0) throw new Exception("open " + st.ToString("X"));
    byte[] mode = System.Text.Encoding.Unicode.GetBytes("ChainingModeGCM\0");
    st = BCryptSetProperty(alg, "ChainingMode", mode, mode.Length, 0); if (st != 0) throw new Exception("mode " + st.ToString("X"));
    st = BCryptGenerateSymmetricKey(alg, out k, IntPtr.Zero, 0, key, key.Length, 0); if (st != 0) throw new Exception("key " + st.ToString("X"));
    GCHandle hn = GCHandle.Alloc(nonce, GCHandleType.Pinned), ht = GCHandle.Alloc(tag, GCHandleType.Pinned);
    INFO i = new INFO(); i.cbSize = Marshal.SizeOf(typeof(INFO)); i.dwInfoVersion = 1;
    i.pbNonce = hn.AddrOfPinnedObject(); i.cbNonce = nonce.Length; i.pbTag = ht.AddrOfPinnedObject(); i.cbTag = tag.Length;
    byte[] o = new byte[ct.Length]; int r;
    st = BCryptDecrypt(k, ct, ct.Length, ref i, null, 0, o, o.Length, out r, 0);
    hn.Free(); ht.Free();
    if (st != 0) throw new Exception("decrypt " + st.ToString("X"));
    return o;
  }
  // minimal protobuf reader: returns list of (field, wiretype, value bytes or varint)
  public static ulong Varint(byte[] b, ref int p) { ulong v = 0; int s = 0; while (true) { byte x = b[p++]; v |= (ulong)(x & 0x7f) << s; if ((x & 0x80) == 0) break; s += 7; } return v; }
  public static List<object[]> Fields(byte[] b, int start, int end) {
    var l = new List<object[]>(); int p = start;
    while (p < end) {
      ulong tagv = Varint(b, ref p); int f = (int)(tagv >> 3), w = (int)(tagv & 7);
      if (w == 0) { l.Add(new object[] { f, w, Varint(b, ref p) }); }
      else if (w == 2) { int len = (int)Varint(b, ref p); byte[] v = new byte[len]; Array.Copy(b, p, v, 0, len); p += len; l.Add(new object[] { f, w, v }); }
      else if (w == 5) { p += 4; } else if (w == 1) { p += 8; } else throw new Exception("wiretype " + w);
    }
    return l;
  }
}
'@
function F($bytes, $n) { foreach ($x in [Gcm]::Fields($bytes, 0, $bytes.Length)) { if ($x[0] -eq $n) { return ,$x[2] } } }
$d = [IO.File]::ReadAllBytes($Vdat)
$outer = F $d 1
$hdr = F $outer 1; $ct = F $outer 2
$enc = F $hdr 2
$keyId = [Text.Encoding]::ASCII.GetString((F $enc 2)); $nonce = F $enc 3; $tag = F $enc 4
"keyId=$keyId nonce=$($nonce.Length) tag=$($tag.Length) ct=$($ct.Length)"
$blob = [IO.File]::ReadAllBytes("$env:LOCALAPPDATA\Ubisoft Game Launcher\store\$keyId.bin")
$key = [Security.Cryptography.ProtectedData]::Unprotect($blob, $null, 'CurrentUser')
"key length $($key.Length)"
$pt = [Gcm]::Decrypt($key, $nonce, $tag, $ct)
[IO.File]::WriteAllBytes($Out, $pt)
"plaintext $($pt.Length) bytes -> $Out; head: " + [Text.Encoding]::UTF8.GetString($pt, 0, [Math]::Min(80, $pt.Length))
