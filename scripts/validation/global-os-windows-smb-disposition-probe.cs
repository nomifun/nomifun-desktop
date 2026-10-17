// Independent control-file experiment, never a product result or recovery.
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;

public static class NomiSmbDispositionControl {
  // Windows SDK: minwinbase.h FILE_INFO_BY_HANDLE_CLASS and winbase.h flags.
  const uint Delete = 0x10000, ReadAttributes = 0x80, Synchronize = 0x100000;
  const uint ShareReadWrite = 3, ShareAll = 7, OpenReparse = 0x200000;
  const int FileIdInfo = 18, FileDispositionInfo = 4, FileDispositionInfoEx = 21;
  static readonly IntPtr Invalid = new IntPtr(-1);
  [StructLayout(LayoutKind.Sequential)] struct IdInfo {
    public ulong Volume;
    [MarshalAs(UnmanagedType.ByValArray, SizeConst=16)] public byte[] Id;
  }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr CreateFileW(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool GetFileInformationByHandleEx(IntPtr file, int kind, out IdInfo info, uint size);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool SetFileInformationByHandle(IntPtr file, int kind, ref uint info, uint size);
  [DllImport("kernel32.dll", EntryPoint="SetFileInformationByHandle", SetLastError=true)]
  static extern bool SetBasicDisposition(IntPtr file, int kind, ref byte info, uint size);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern uint GetFileAttributesW(string path);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool CloseHandle(IntPtr file);
  static Dictionary<string,object> Row(string operation, bool success, int error) {
    return new Dictionary<string,object>{{"operation",operation},{"success",success},{"raw_os_error",error}};
  }
  static Dictionary<string,object> NameState(string path) {
    uint attributes=GetFileAttributesW(path); int error=attributes==0xffffffff ? Marshal.GetLastWin32Error() : 0;
    return new Dictionary<string,object>{{"attributes",attributes==0xffffffff ? (object)null : attributes},
      {"raw_os_error",error},{"absent",error==2 || error==3}};
  }
  static object Identity(IntPtr handle, List<object> steps) {
    IdInfo value; bool ok=GetFileInformationByHandleEx(handle,FileIdInfo,out value,24);
    int error=ok ? 0 : Marshal.GetLastWin32Error(); steps.Add(Row("FileIdInfo",ok,error));
    return ok ? new Dictionary<string,object>{{"volume",value.Volume.ToString("x16")},
      {"file_id",BitConverter.ToString(value.Id).Replace("-","").ToLowerInvariant()}} : null;
  }
  static void Close(ref IntPtr handle, string operation, List<object> steps) {
    if(handle==Invalid) return;
    bool ok=CloseHandle(handle); int error=ok ? 0 : Marshal.GetLastWin32Error();
    steps.Add(Row(operation,ok,error)); if(ok) handle=Invalid;
  }
  static void RequireDirectory(string root, bool ancestors) {
    for(string path=root; path!=null; path=ancestors ? Path.GetDirectoryName(path) : null) {
      uint attributes=GetFileAttributesW(path);
      if(attributes==0xffffffff) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(),"control directory metadata unavailable");
      if((attributes&0x10)==0 || (attributes&0x400)!=0) throw new InvalidOperationException("control directory changed type or became a reparse point");
    }
  }
  public static object Run(string nonce, string local, string unc, string markerText) {
    RequireDirectory(local,true); RequireDirectory(unc,false);
    string marker="identity-"+nonce+".txt";
    if(File.ReadAllText(Path.Combine(local,marker))!=markerText || File.ReadAllText(Path.Combine(unc,marker))!=markerText)
      throw new InvalidOperationException("independent share marker mismatch");
    string[] transports={"local","unc"}; uint[] flags={0x13,0x03,0x01,0};
    // Check all names before the first create, then use CREATE_NEW for every
    // case. No existing file, original sample or recovery backup is opened.
    foreach(string transport in transports) foreach(bool pinned in new[]{false,true}) foreach(uint flag in flags) {
      string name="control-"+nonce+"-"+transport+"-"+(pinned ? "pinned" : "unheld")+"-"+flag.ToString("x2")+".bin";
      uint attributes=GetFileAttributesW(Path.Combine(local,name));
      if(attributes!=0xffffffff || Marshal.GetLastWin32Error()!=2) throw new InvalidOperationException("control filename is not genuinely absent");
    }
    var cases=new List<object>();
    foreach(string transport in transports) foreach(bool pinned in new[]{false,true}) foreach(uint flag in flags) {
      string name="control-"+nonce+"-"+transport+"-"+(pinned ? "pinned" : "unheld")+"-"+flag.ToString("x2")+".bin";
      string path=Path.Combine(transport=="local" ? local : unc,name);
      var steps=new List<object>();
      var result=new Dictionary<string,object>{{"name",name},{"transport",transport},{"metadata_holder",pinned},
        {"information_class",flag==0 ? FileDispositionInfo : FileDispositionInfoEx},{"flags",flag==0 ? (object)null : flag},
        {"open_access",Delete|ReadAttributes|Synchronize},{"open_sharing",ShareReadWrite},{"steps",steps},{"marked",false}};
      IntPtr created=Invalid,holder=Invalid,current=Invalid;
      try {
        created=CreateFileW(path,ReadAttributes|Synchronize,ShareAll,IntPtr.Zero,1,OpenReparse|0x80,IntPtr.Zero);
        int error=created==Invalid ? Marshal.GetLastWin32Error() : 0;
        steps.Add(Row("CREATE_NEW",created!=Invalid,error));
        if(created!=Invalid) {
          result["created_identity"]=Identity(created,steps); Close(ref created,"close_created",steps);
          if(created!=Invalid) throw new InvalidOperationException("control creation handle did not close");
          if(pinned) {
            holder=CreateFileW(path,ReadAttributes|Synchronize,ShareAll,IntPtr.Zero,3,OpenReparse,IntPtr.Zero);
            error=holder==Invalid ? Marshal.GetLastWin32Error() : 0;
            steps.Add(Row("open_metadata_holder",holder!=Invalid,error));
            if(holder!=Invalid) result["holder_identity"]=Identity(holder,steps);
          }
          if(!pinned || holder!=Invalid) {
            current=CreateFileW(path,Delete|ReadAttributes|Synchronize,ShareReadWrite,IntPtr.Zero,3,OpenReparse,IntPtr.Zero);
            error=current==Invalid ? Marshal.GetLastWin32Error() : 0;
            steps.Add(Row("open_delete_control",current!=Invalid,error));
            if(current!=Invalid) {
              result["delete_identity"]=Identity(current,steps);
              uint info=flag; byte deleteFile=1;
              bool marked=flag==0 ? SetBasicDisposition(current,FileDispositionInfo,ref deleteFile,1)
                : SetFileInformationByHandle(current,FileDispositionInfoEx,ref info,4);
              error=marked ? 0 : Marshal.GetLastWin32Error(); result["marked"]=marked;
              steps.Add(Row("set_disposition",marked,error)); result["name_after_mark"]=NameState(path);
            }
          }
        }
      } catch(Exception error) {
        var native=error as System.ComponentModel.Win32Exception;
        result["exception"]=new Dictionary<string,object>{{"kind",error.GetType().FullName},
          {"message",error.Message},{"raw_os_error",native==null ? (object)null : native.NativeErrorCode}};
      } finally {
        Close(ref current,"close_delete_control",steps);
        result["name_after_delete_handle_close_attempt"]=NameState(path);
        Close(ref holder,"close_metadata_holder",steps); Close(ref created,"close_created_finally",steps);
        bool allClosed=current==Invalid && holder==Invalid && created==Invalid;
        result["all_own_handles_closed"]=allClosed;
        result["name_after_all_own_handles_closed"]=allClosed ? NameState(path) : null;
        result["local_name_after_all_own_handles_closed"]=allClosed ? NameState(Path.Combine(local,name)) : null;
        result["unc_name_after_all_own_handles_closed"]=allClosed ? NameState(Path.Combine(unc,name)) : null;
      }
      cases.Add(result);
    }
    return new Dictionary<string,object>{{"schema","nomifun.windows-smb-disposition-control.v1"},{"nonce",nonce},
      {"cases",cases},{"automatic_fallbacks",0},{"manual_file_cleanup",0},{"original_effect_settlement",false},
      {"scope","Independent control files only; not product acceptance, original outcome proof or reconciliation"}};
  }
}
