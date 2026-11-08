using System;
using System.IO;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;
using System.Text;
class OleDropProbe {
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int X,Y;}
 [StructLayout(LayoutKind.Sequential)] public struct Rect {public int L,T,R,B;}
 [StructLayout(LayoutKind.Sequential)] struct Mouse {public int X,Y;public uint Data,Flags,Time;public UIntPtr Extra;}
 [StructLayout(LayoutKind.Sequential)] struct Input {public uint Type;public Mouse Mouse;}
 delegate bool EnumProc(IntPtr h,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr p);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
 [DllImport("user32.dll")] static extern IntPtr GetProp(IntPtr h,string n);
 [DllImport("user32.dll")] static extern IntPtr GetWindowLongPtr(IntPtr h,int n);
 [DllImport("user32.dll")] static extern int GetClassName(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] static extern bool GetCursorPos(out Point p);
 [DllImport("user32.dll")] static extern uint SendInput(uint n,Input[] input,int size);
 static int targetPid;static Rect rect;static Stopwatch clock=new Stopwatch();static volatile bool cancel;
 static void Send(int x,int y,uint flags) {var i=new Input(); i.Mouse.X=x*65535/(Screen.PrimaryScreen.Bounds.Width-1);i.Mouse.Y=y*65535/(Screen.PrimaryScreen.Bounds.Height-1);i.Mouse.Flags=flags; if(SendInput(1,new[]{i},Marshal.SizeOf(typeof(Input)))!=1)throw new Exception("SendInput failed");}
 static void Move(int x,int y){Send(x,y,0x8001);}
 static void Describe(IntPtr h) {uint pid;GetWindowThreadProcessId(h,out pid);var s=new StringBuilder(256);GetClassName(h,s,256); Console.Write(" hwnd=0x{0:x} pid={1} class={2} registered={3} transparent={4}",h.ToInt64(),pid,s,GetProp(h,"OleDropTargetInterface")!=IntPtr.Zero,(GetWindowLongPtr(h,-20).ToInt64()&32)!=0);}
 [STAThread] static int Main(string[] args) {
 if(args.Length<2){Console.Error.WriteLine("usage: ole-drop-probe.exe DEV_PID FILE [drop-y=120] [idle-seconds=35]");return 2;}
 targetPid=int.Parse(args[0]);var proc=Process.GetProcessById(targetPid);if(!proc.MainModule.FileName.StartsWith(@"F:\tuck-agent-target\",StringComparison.OrdinalIgnoreCase))throw new Exception("Refusing non-agent target");
 IntPtr root=IntPtr.Zero;EnumWindows((h,p)=>{uint pid;GetWindowThreadProcessId(h,out pid);Rect r;GetWindowRect(h,out r);if(pid==targetPid&&r.R-r.L>=500&&r.B-r.T>=280){root=h;rect=r;}return true;},IntPtr.Zero);if(root==IntPtr.Zero)throw new Exception("island not found");
 Console.WriteLine("target={0} path={1} rect={2},{3},{4},{5}",targetPid,proc.MainModule.FileName,rect.L,rect.T,rect.R,rect.B);Describe(root);Console.WriteLine();EnumChildWindows(root,(h,p)=>{Describe(h);Console.WriteLine();return true;},IntPtr.Zero);
 int x=(rect.L+rect.R)/2,dy=args.Length>2?int.Parse(args[2]):120,idle=args.Length>3?int.Parse(args[3]):35;Move(x,rect.B+100);Console.WriteLine("idle seconds={0}",idle);Thread.Sleep(idle*1000);
 var f=new Form();f.Text="OLE file source (agent probe)";f.SetBounds(30,400,240,100);
 f.GiveFeedback+=(s,e)=>{Point pt;GetCursorPos(out pt);Console.Write("t={0} point={1},{2} effect={3}",clock.ElapsedMilliseconds,pt.X,pt.Y,e.Effect);var h=WindowFromPoint(pt);uint pid;GetWindowThreadProcessId(h,out pid);if(pt.Y<rect.B && pt.Y>=rect.T && pid!=targetPid){cancel=true;}for(int j=0;j<6&&h!=IntPtr.Zero;j++,h=GetParent(h))Describe(h);Console.WriteLine();e.UseDefaultCursors=true;};
 f.QueryContinueDrag+=(s,e)=>{if(cancel)e.Action=DragAction.Cancel;};
 f.Shown+=(s,e)=>{var data=new DataObject(DataFormats.FileDrop,new[]{Path.GetFullPath(args[1])});Move(x,rect.B+100);Send(0,0,2);clock.Start();new Thread(()=>{Thread.Sleep(400);Move(x,rect.T+20);Thread.Sleep(1500);for(int y=25;y<=dy;y+=5){Move(x,rect.T+y);Thread.Sleep(120);}Thread.Sleep(700);Send(0,0,4);}){IsBackground=true}.Start();var result=f.DoDragDrop(data,DragDropEffects.Copy);Console.WriteLine("RESULT effect={0} cancelled-for-foreign-window={1}",result,cancel);Move(x,rect.B+100);f.Close();};
 Application.Run(f);return cancel?3:0;
 }
}
