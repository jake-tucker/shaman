# Writes a shortcut as install.ps1 would, with WScript.Shell, and then, when asked, sets System.AppUserModel.ID on it
# through the shell's property store, which WScript.Shell cannot do. Works in Windows PowerShell 5.1 with nothing
# installed. With -Read it only says what a shortcut holds. Issue #801.
param(
    [Parameter(Mandatory)][string]$Path,
    [string]$Target = '',
    [string]$Icon = '',
    [string]$AppId = '',
    [switch]$Read
)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Link801 {
  [StructLayout(LayoutKind.Sequential, Pack = 4)] public struct PROPERTYKEY { public Guid fmtid; public uint pid; }
  [StructLayout(LayoutKind.Explicit, Size = 24)] public struct PROPVARIANT { [FieldOffset(0)] public ushort vt; [FieldOffset(8)] public IntPtr p; }
  [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IPropertyStore {
    int GetCount(out uint n);
    int GetAt(uint i, out PROPERTYKEY key);
    int GetValue(ref PROPERTYKEY key, out PROPVARIANT v);
    int SetValue(ref PROPERTYKEY key, ref PROPVARIANT v);
    int Commit();
  }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHGetPropertyStoreFromParsingName(string path, IntPtr ctx, int flags, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IPropertyStore store);
  [DllImport("ole32.dll")] static extern int PropVariantClear(ref PROPVARIANT v);
  static PROPERTYKEY Key() { PROPERTYKEY k; k.fmtid = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"); k.pid = 5; return k; }
  static IPropertyStore Open(string path, int flags) { Guid iid = new Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"); IPropertyStore s; SHGetPropertyStoreFromParsingName(path, IntPtr.Zero, flags, ref iid, out s); return s; }
  public static void SetAppId(string path, string id) {
    IPropertyStore s = Open(path, 2);   // GPS_READWRITE
    PROPERTYKEY k = Key(); PROPVARIANT v = new PROPVARIANT(); v.vt = 31; v.p = Marshal.StringToCoTaskMemUni(id);   // VT_LPWSTR
    try { Marshal.ThrowExceptionForHR(s.SetValue(ref k, ref v)); Marshal.ThrowExceptionForHR(s.Commit()); }
    finally { PropVariantClear(ref v); Marshal.ReleaseComObject(s); }
  }
  public static string GetAppId(string path) {
    IPropertyStore s = Open(path, 0);
    PROPERTYKEY k = Key(); PROPVARIANT v;
    try { Marshal.ThrowExceptionForHR(s.GetValue(ref k, out v)); string r = v.vt == 31 ? Marshal.PtrToStringUni(v.p) : null; PropVariantClear(ref v); return r; }
    finally { Marshal.ReleaseComObject(s); }
  }
}
"@
if (-not $Read) {
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($Path)
    $link.TargetPath = $Target
    $link.WorkingDirectory = Split-Path $Target
    if ($Icon) { $link.IconLocation = "$Icon,0" }
    $link.Save()
    if ($AppId) { [Link801]::SetAppId($Path, $AppId) }
}
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($Path)
"shortcut $Path -> $($link.TargetPath), icon $($link.IconLocation), AppUserModelID '$([Link801]::GetAppId($Path))'"
