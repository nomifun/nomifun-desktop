// Task-owned acceptance fixture. Build only with the existing .NET Framework
// compiler; no installer, privileges, keyboard/mouse, clipboard or screenshots.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;

internal static class Native {
    [StructLayout(LayoutKind.Sequential)] internal struct FileTime { public uint Low,High; }
    [DllImport("kernel32.dll",SetLastError=true)] internal static extern IntPtr OpenProcess(uint rights,bool inherit,int pid);
    [DllImport("kernel32.dll")] internal static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] internal static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
    [DllImport("kernel32.dll")] internal static extern bool GetExitCodeProcess(IntPtr handle,out uint code);
    [DllImport("kernel32.dll")] internal static extern bool GetProcessTimes(IntPtr handle,out FileTime creation,out FileTime exit,out FileTime kernel,out FileTime user);
    [DllImport("kernel32.dll")] internal static extern bool IsProcessInJob(IntPtr process,IntPtr job,out bool value);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] internal static extern bool QueryFullProcessImageName(IntPtr process,uint flags,StringBuilder path,ref int length);
    [DllImport("user32.dll")] internal static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] internal static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
    [DllImport("user32.dll")] internal static extern bool PostMessage(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    internal static string Creation(IntPtr handle) { FileTime c,e,k,u;if(!GetProcessTimes(handle,out c,out e,out k,out u))throw new Win32Exception();return (((ulong)c.High<<32)|c.Low).ToString(); }
    internal static bool Alive(IntPtr handle){return WaitForSingleObject(handle,0)==258;}
    internal static string Image(IntPtr handle){var path=new StringBuilder(32768);int size=path.Capacity;if(!QueryFullProcessImageName(handle,0,path,ref size))throw new Win32Exception();return path.ToString();}
    internal static bool InJob(IntPtr handle){bool value;if(!IsProcessInJob(handle,IntPtr.Zero,out value))throw new Win32Exception();return value;}
}
internal sealed class DedicatedForm:Form { protected override bool ShowWithoutActivation {get{return true;}} }
internal static class Program {
    private static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
    private static void Write(string root,string name,object data){File.WriteAllText(Path.Combine(root,name),Json.Serialize(data));}
    private static Dictionary<string,object> Read(string root,string name){return Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(Path.Combine(root,name)));}
    private static void WaitFile(string root,string name){var until=DateTime.UtcNow.AddSeconds(180);while(!File.Exists(Path.Combine(root,name))){if(File.Exists(Path.Combine(root,"host-error.json")))throw new Exception("host reported a fixture error");if(DateTime.UtcNow>until)throw new TimeoutException(name);Thread.Sleep(10);}}
    private static bool OwnWindow(IntPtr window,int pid){uint owner;return Native.IsWindow(window)&&Native.GetWindowThreadProcessId(window,out owner)!=0&&owner==(uint)pid;}
    private static object EnvironmentEvidence(string root){
        string privateRoot=Path.Combine(root,"host-data");bool leaked=false;
        foreach(System.Collections.DictionaryEntry entry in Environment.GetEnvironmentVariables()){string key=Convert.ToString(entry.Key),value=Convert.ToString(entry.Value);if(value.StartsWith(privateRoot,StringComparison.OrdinalIgnoreCase)||key.StartsWith("CUA_DRIVER_",StringComparison.Ordinal))leaked=true;}
        return new {private_driver_environment_leaked=leaked,user_profile_matches_token=String.Equals(Environment.GetEnvironmentVariable("USERPROFILE"),Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),StringComparison.OrdinalIgnoreCase),current_directory_is_private=Environment.CurrentDirectory.StartsWith(privateRoot,StringComparison.OrdinalIgnoreCase)};
    }
    [STAThread] private static int Main(string[] args){
        if(args.Length!=2)return 2;
        string root=Path.GetFullPath(args[1]);
        try {if(args[0]=="gui"||args[0]=="functional"){Gui(root,args[0]=="functional");return 0;}if(args[0]=="observe"){Observe(root);return 0;}return 2;}
        catch(Exception error){Write(root,"fixture-error.json",new {error=error.Message,mode=args[0]});return 1;}
    }
    private static void Gui(string root,bool visible){
        Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
        var form=new DedicatedForm();form.Text="NomiFun Lifetime Test "+Path.GetFileName(root);form.ClientSize=new Size(510,300);form.WindowState=visible?FormWindowState.Normal:FormWindowState.Minimized;
        if(visible){
            string visualCode=Guid.NewGuid().ToString("N").Substring(0,8).ToUpperInvariant();
            Write(root,"visual-marker.json",new {code=visualCode,pid=Process.GetCurrentProcess().Id});
            form.Paint+=delegate(object sender,PaintEventArgs paint){using(var font=new Font(FontFamily.GenericMonospace,12,FontStyle.Bold)){paint.Graphics.DrawString(visualCode,font,Brushes.Black,new PointF(260,245));}};
        }
        form.Controls.Add(new Label {Text="Dedicated Computer Use test. Closes automatically.",Location=new Point(12,10),Size=new Size(480,30)});
        var display=new TextBox {Name="OwnedCalculation",AccessibleName="Owned calculation",ReadOnly=true,Text="0",Location=new Point(12,45),Size=new Size(470,28)};form.Controls.Add(display);
        string[] labels={"2","+","3","="},names={"Owned two","Owned add","Owned three","Owned equals"};
        for(int index=0;index<labels.Length;index++){string label=labels[index];var button=new Button {Text=label,AccessibleName=names[index],Location=new Point(12+index*95,82),Size=new Size(85,35)};button.Click+=delegate{if(label=="2")display.Text="2";if(label=="+")display.Text="2+";if(label=="3")display.Text="2+3";if(label=="="){display.Text="5";Write(root,"calculation.json",new {expression="2+3",result=display.Text});}};form.Controls.Add(button);}
        var editor=new TextBox {Name="OwnedEditor",AccessibleName="Owned editor",Multiline=true,Location=new Point(12,130),Size=new Size(470,95)};form.Controls.Add(editor);
        var save=new Button {Text="Save owned test text",AccessibleName="Save owned test text",Location=new Point(12,240),Size=new Size(230,35)};save.Click+=delegate{File.WriteAllText(Path.Combine(root,"saved.txt"),editor.Text,new UTF8Encoding(false));Write(root,"saved.json",new {characters=editor.Text.Length,path="saved.txt"});};form.Controls.Add(save);
        var timer=new System.Windows.Forms.Timer();timer.Interval=50;
        timer.Tick+=delegate{if(File.Exists(Path.Combine(root,"close.request")))form.Close();};
        form.Shown+=delegate{using(var process=Process.GetCurrentProcess()){Write(root,"ready.json",new {pid=process.Id,window=form.Handle.ToInt64().ToString(),dpi=Native.GetDpiForWindow(form.Handle),creation_filetime=Native.Creation(process.Handle),executable=Application.ExecutablePath,in_job=Native.InJob(process.Handle),title=form.Text,environment=EnvironmentEvidence(root)});}timer.Start();};
        form.FormClosed+=delegate{timer.Stop();Write(root,"exit.json",new {pid=Process.GetCurrentProcess().Id,graceful=true,control_file=File.Exists(Path.Combine(root,"close.request"))});};
        Application.Run(form);timer.Dispose();form.Dispose();
    }
    private static object State(IntPtr app,IntPtr worker,IntPtr window,int pid){uint exit;Native.GetExitCodeProcess(app,out exit);return new {app_alive=Native.Alive(app),app_window=OwnWindow(window,pid),app_exit_code=exit,worker_alive=Native.Alive(worker)};}
    private static void Observe(string root){
        WaitFile(root,"launched.json");WaitFile(root,"ready.json");var ready=Read(root,"ready.json");var launch=Read(root,"launched.json");
        int appPid=Convert.ToInt32(ready["pid"]),workerPid=Convert.ToInt32(launch["worker_pid"]);var window=new IntPtr(Convert.ToInt64(ready["window"]));
        const uint rights=0x100000|0x0400|0x1000;
        IntPtr app=Native.OpenProcess(rights,false,appPid),worker=Native.OpenProcess(rights,false,workerPid);
        if(app==IntPtr.Zero||worker==IntPtr.Zero)throw new Win32Exception();
        try{
            if(Native.Creation(app)!=Convert.ToString(ready["creation_filetime"])||!String.Equals(Native.Image(app),Application.ExecutablePath,StringComparison.OrdinalIgnoreCase))throw new Exception("owned application identity mismatch");
            var before=State(app,worker,window,appPid);bool appInJob=Native.InJob(app),workerInJob=Native.InJob(worker);string appCreation=Native.Creation(app),workerCreation=Native.Creation(worker);
            File.WriteAllText(Path.Combine(root,"observer-ready"),"exact handles retained");
            WaitFile(root,"session-closed.json");var afterClose=State(app,worker,window,appPid);
            WaitFile(root,"retired.json");Thread.Sleep(100);var afterIdle=State(app,worker,window,appPid);
            WaitFile(root,"finished.json");var afterShutdown=State(app,worker,window,appPid);bool survived=Native.Alive(app)&&OwnWindow(window,appPid);
            File.WriteAllText(Path.Combine(root,"close.request"),"close only this fixture");
            if(Native.Alive(app)&&Native.WaitForSingleObject(app,5000)==258&&OwnWindow(window,appPid))Native.PostMessage(window,0x10,IntPtr.Zero,IntPtr.Zero);
            bool closed=Native.WaitForSingleObject(app,5000)==0;
            Write(root,"observer-report.json",new {ok=true,survived_worker_retirement=survived,app_pid=appPid,worker_pid=workerPid,app_creation_filetime=appCreation,worker_creation_filetime=workerCreation,app_in_job=appInJob,worker_in_job=workerInJob,before=before,after_session_close=afterClose,after_idle=afterIdle,after_shutdown=afterShutdown,environment=ready["environment"],dpi=ready["dpi"],fixture_closed=closed,graceful_exit_record=File.Exists(Path.Combine(root,"exit.json")),forced_stop=false,screenshots=0,input_injection=0});
        }finally{Native.CloseHandle(app);Native.CloseHandle(worker);File.WriteAllText(Path.Combine(root,"close.request"),"close only this fixture");}
    }
}
