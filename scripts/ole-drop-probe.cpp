#include <chrono>
#include <cstdio>
#include <ole2.h>
#include <shellapi.h>
#include <shlobj.h>
#include <string>
#include <thread>
#include <windows.h>
static DWORD pid;
static HWND island;
static RECT box;
static bool foreign;
static ULONGLONG started;
static void describe(HWND h) {
  DWORD p = 0;
  GetWindowThreadProcessId(h, &p);
  wchar_t name[256] = {};
  GetClassNameW(h, name, 256);
  printf(" hwnd=%p pid=%lu class=%ls registered=%p transparent=%d", h, p, name,
         GetPropW(h, L"OleDropTargetInterface"),
         (GetWindowLongPtrW(h, GWL_EXSTYLE) & WS_EX_TRANSPARENT) != 0);
  printf(" exstyle=%llx dpi=%u",
         (unsigned long long)GetWindowLongPtrW(h, GWL_EXSTYLE),
         GetDpiForWindow(h));
  HRGN region = CreateRectRgn(0, 0, 0, 0);
  RECT bounds = {};
  int kind = GetWindowRgn(h, region);
  if (kind > 0) {
    GetRgnBox(region, &bounds);
    printf(" region=%d:[%ld,%ld,%ld,%ld]", kind, bounds.left, bounds.top,
           bounds.right, bounds.bottom);
  }
  DeleteObject(region);
}
static void move(int x, int y,
                 DWORD flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE) {
  INPUT i = {};
  i.type = INPUT_MOUSE;
  i.mi.dx = MulDiv(x, 65535, GetSystemMetrics(SM_CXSCREEN) - 1);
  i.mi.dy = MulDiv(y, 65535, GetSystemMetrics(SM_CYSCREEN) - 1);
  i.mi.dwFlags = flags;
  if (SendInput(1, &i, sizeof(i)) != 1) {
    printf("SendInput failed %lu\n", GetLastError());
    ExitProcess(4);
  }
}
struct Data : IDataObject {
  std::wstring path;
  ULONG refs = 1;
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **out) override {
    *out = nullptr;
    if (id == IID_IUnknown || id == IID_IDataObject) {
      *out = this;
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
  ULONG STDMETHODCALLTYPE Release() override { return --refs; }
  HRESULT STDMETHODCALLTYPE GetData(FORMATETC *f, STGMEDIUM *m) override {
    if (QueryGetData(f) != S_OK)
      return DV_E_FORMATETC;
    size_t n = (path.size() + 2) * sizeof(wchar_t);
    HGLOBAL h = GlobalAlloc(GHND, sizeof(DROPFILES) + n);
    if (!h)
      return E_OUTOFMEMORY;
    auto d = (DROPFILES *)GlobalLock(h);
    d->pFiles = sizeof(DROPFILES);
    d->fWide = TRUE;
    memcpy((char *)d + sizeof(DROPFILES), path.c_str(),
           (path.size() + 1) * sizeof(wchar_t));
    GlobalUnlock(h);
    m->tymed = TYMED_HGLOBAL;
    m->hGlobal = h;
    m->pUnkForRelease = nullptr;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE QueryGetData(FORMATETC *f) override {
    return f->cfFormat == CF_HDROP && (f->tymed & TYMED_HGLOBAL)
               ? S_OK
               : DV_E_FORMATETC;
  }
  HRESULT STDMETHODCALLTYPE GetDataHere(FORMATETC *, STGMEDIUM *) override {
    return E_NOTIMPL;
  }
  HRESULT STDMETHODCALLTYPE GetCanonicalFormatEtc(FORMATETC *,
                                                  FORMATETC *o) override {
    o->ptd = nullptr;
    return E_NOTIMPL;
  }
  HRESULT STDMETHODCALLTYPE SetData(FORMATETC *, STGMEDIUM *, BOOL) override {
    return E_NOTIMPL;
  }
  HRESULT STDMETHODCALLTYPE EnumFormatEtc(DWORD d,
                                          IEnumFORMATETC **e) override {
    FORMATETC f = {CF_HDROP, nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    return d == DATADIR_GET ? SHCreateStdEnumFmtEtc(1, &f, e) : E_NOTIMPL;
  }
  HRESULT STDMETHODCALLTYPE DAdvise(FORMATETC *, DWORD, IAdviseSink *,
                                    DWORD *) override {
    return OLE_E_ADVISENOTSUPPORTED;
  }
  HRESULT STDMETHODCALLTYPE DUnadvise(DWORD) override {
    return OLE_E_ADVISENOTSUPPORTED;
  }
  HRESULT STDMETHODCALLTYPE EnumDAdvise(IEnumSTATDATA **) override {
    return OLE_E_ADVISENOTSUPPORTED;
  }
};
static bool over_island() {
  POINT p;
  GetCursorPos(&p);
  if (p.x < box.left || p.x >= box.right || p.y < box.top || p.y >= box.bottom)
    return false;
  for (HWND h = WindowFromPoint(p); h; h = GetParent(h))
    if (h == island)
      return true;
  return false;
}
struct Source : IDropSource {
  ULONG refs = 1;
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **out) override {
    *out = nullptr;
    if (id == IID_IUnknown || id == IID_IDropSource) {
      *out = this;
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
  ULONG STDMETHODCALLTYPE Release() override { return --refs; }
  HRESULT STDMETHODCALLTYPE QueryContinueDrag(BOOL escape,
                                              DWORD keys) override {
    if (escape)
      return DRAGDROP_S_CANCEL;
    if (!(keys & MK_LBUTTON))
      return over_island() ? DRAGDROP_S_DROP : DRAGDROP_S_CANCEL;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GiveFeedback(DWORD effect) override {
    POINT pt;
    GetCursorPos(&pt);
    HWND leaf = WindowFromPoint(pt);
    bool ours = false;
    for (HWND h = leaf; h; h = GetParent(h))
      if (h == island)
        ours = true;
    foreign = !ours;
    printf("t=%llu point=%ld,%ld effect=%lu", GetTickCount64() - started, pt.x,
           pt.y, effect);
    for (HWND h = leaf; h; h = GetParent(h))
      describe(h);
    printf(" islandTransparent=%d hit=%lldx%lld", (GetWindowLongPtrW(island,GWL_EXSTYLE)&WS_EX_TRANSPARENT)!=0,
      (long long)GetPropW(island,L"TuckProbeHitWidth"),(long long)GetPropW(island,L"TuckProbeHitHeight"));
    puts("");
    return DRAGDROP_S_USEDEFAULTCURSORS;
  }
};
int wmain(int argc, wchar_t **argv) {
  setvbuf(stdout, nullptr, _IONBF, 0);
  if (argc < 3)
    return 2;
  SetProcessDPIAware();
  pid = wcstoul(argv[1], nullptr, 10);
  HANDLE proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
  wchar_t exe[32768];
  DWORD n = 32768;
  if (!proc || !QueryFullProcessImageNameW(proc, 0, exe, &n) ||
      _wcsnicmp(exe, L"F:\\tuck-agent-target\\", 21) != 0) {
    puts("Refusing non-agent target");
    return 2;
  }
  CloseHandle(proc);
  EnumWindows(
      [](HWND h, LPARAM) -> BOOL {
        DWORD p;
        GetWindowThreadProcessId(h, &p);
        wchar_t title[256];
        GetWindowTextW(h, title, 256);
        if (p == pid && wcscmp(title, L"Tuck Drop Probe") == 0)
          island = h;
        return TRUE;
      },
      0);
  if (!island)
    return 2;
  GetWindowRect(island, &box);
  printf("target=%lu rect=%ld,%ld,%ld,%ld\n", pid, box.left, box.top, box.right,
         box.bottom);
  describe(island);
  puts("");
  EnumChildWindows(
      island,
      [](HWND h, LPARAM) -> BOOL {
        describe(h);
        puts("");
        return TRUE;
      },
      0);
  int x = (box.left + box.right) / 2, dy = argc > 3 ? _wtoi(argv[3]) : 120,
      idle = argc > 4 ? _wtoi(argv[4]) : 35;
  move(x, box.bottom + 100);
  printf("idle seconds=%d\n", idle);
  Sleep(idle * 1000);
  if (argc > 5 && wcscmp(argv[5], L"far") == 0) {
    move(GetSystemMetrics(SM_CXSCREEN) - 40,
         GetSystemMetrics(SM_CYSCREEN) - 40);
    puts("CURSOR FAR");
    return 0;
  }
  if (argc > 5 && wcscmp(argv[5], L"geometry") == 0) {
    move(x, box.top + 20);
    Sleep(100);
    printf("PILL hit=%d", over_island());
    describe(island);
    puts("");
    Sleep(1000);
    printf("HOVER OPEN");
    describe(island);
    puts("");
    move(x, box.bottom + 100);
    Sleep(800);
    printf("OUTSIDE hit=%d", over_island());
    describe(island);
    puts("");
    return over_island() ? 6 : 0;
  }
  HRESULT init = OleInitialize(nullptr);
  printf("OleInitialize=%lx\n", init);
  if (FAILED(init))
    return 2;
  puts("initialized");
  WNDCLASSW wc = {};
  wc.lpfnWndProc = DefWindowProcW;
  wc.hInstance = GetModuleHandleW(nullptr);
  wc.lpszClassName = L"TuckOleSource";
  RegisterClassW(&wc);
  HWND sourceWindow = CreateWindowW(
      wc.lpszClassName, L"Agent OLE source", WS_OVERLAPPEDWINDOW | WS_VISIBLE,
      30, 400, 260, 120, nullptr, nullptr, wc.hInstance, nullptr);
  SetForegroundWindow(sourceWindow);
  MSG message;
  while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
    TranslateMessage(&message);
    DispatchMessageW(&message);
  }
  if (argc > 5 && wcscmp(argv[5], L"open") == 0) {
    move(x, box.top + 20);
    Sleep(1200);
  }
  HWND lateChild = nullptr;
  if (argc > 5 && wcscmp(argv[5], L"late-child") == 0) {
    lateChild = CreateWindowW(L"STATIC", L"", WS_CHILD | WS_VISIBLE, 0, 0,
                              box.right - box.left, box.bottom - box.top,
                              island, nullptr, wc.hInstance, nullptr);
    printf("late-child hwnd=%p error=%lu\n", lateChild, GetLastError());
    // OLE registration marshals messages to the child HWND's owning thread.
    // Pump that thread while waiting; sleeping here can block the registration.
    DWORD deadline = GetTickCount() + 3000;
    do {
      while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
      }
      if (GetPropW(lateChild, L"OleDropTargetInterface"))
        break;
      Sleep(10);
    } while (GetTickCount() < deadline);
    printf("late-child registered=%p\n",
           GetPropW(lateChild, L"OleDropTargetInterface"));
    if (!lateChild || !GetPropW(lateChild, L"OleDropTargetInterface")) {
      if (lateChild)
        DestroyWindow(lateChild);
      DestroyWindow(sourceWindow);
      OleUninitialize();
      return 5;
    }
    DestroyWindow(lateChild);
    DestroyWindow(sourceWindow);
    OleUninitialize();
    puts("REGISTRATION PASS (before island activity)");
    return 0;
  }
  Data data;
  data.path = argv[2];
  Source source;
  move(100, 450);
  Sleep(120);
  puts("press");
  move(0, 0, MOUSEEVENTF_LEFTDOWN);
  Sleep(100);
  while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
    TranslateMessage(&message);
    DispatchMessageW(&message);
  }
  printf("pressed key=%d async=%d\n", GetKeyState(VK_LBUTTON),
         GetAsyncKeyState(VK_LBUTTON));
  SetCapture(sourceWindow);
  started = GetTickCount64();
  std::thread worker([=]() {
    Sleep(argc > 5 ? 80 : 400);
    move(x, box.top + 20);
    Sleep(argc > 5 && wcscmp(argv[5], L"fast") == 0 ? 0 : 1500);
    for (int y = 25; y <= dy; y += 5) {
      move(x, box.top + y);
      Sleep(argc > 5 && wcscmp(argv[5], L"fast") == 0 ? 1 : 120);
    }
    Sleep(argc > 5 && wcscmp(argv[5], L"fast") == 0 ? 0 : 700);
    move(0, 0, MOUSEEVENTF_LEFTUP);
  });
  DWORD effect = 0;
  puts("DoDragDrop");
  HRESULT hr = DoDragDrop(&data, &source, DROPEFFECT_COPY, &effect);
  worker.join();
  foreign = !over_island();
  printf("RESULT hr=%lx effect=%lu foreign=%d\n", hr, effect, foreign);
  move(x, box.bottom + 100);
  if (lateChild)
    DestroyWindow(lateChild);
  ReleaseCapture();
  DestroyWindow(sourceWindow);
  OleUninitialize();
  return hr == DRAGDROP_S_DROP && effect == DROPEFFECT_COPY && !foreign ? 0 : 1;
}
