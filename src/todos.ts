import { newRequestId } from './live-state.js';
export interface Todo { id: string; text: string; done: boolean; dueAt: number | null; reminded: boolean }
const KEY = 'tuck.todos.v1';
export function normalizeTodos(input: unknown): Todo[] {
  if (!Array.isArray(input)) return [];
  return input.filter((value): value is Todo => !!value && typeof value === 'object' && typeof value.id === 'string' && typeof value.text === 'string')
    .slice(0, 100).map(value => ({ id: value.id.slice(0, 80), text: value.text.slice(0, 500), done: value.done === true,
      dueAt: typeof value.dueAt === 'number' && Number.isFinite(value.dueAt) && value.dueAt > 0 ? value.dueAt : null, reminded: value.reminded === true }));
}
export function dueTodos(todos: Todo[], now: number): Todo[] {
  return todos.filter(todo => !todo.done && !todo.reminded && todo.dueAt !== null && todo.dueAt <= now);
}
export function mountTodos(): void {
  const page = document.createElement('section'); page.className = 'page live-page todos'; page.dataset.page = 'todos';
  page.setAttribute('aria-label', 'To-dos and reminders'); page.setAttribute('aria-hidden', 'true'); page.inert = true;
  page.innerHTML = '<h2>To-dos</h2><form><input type="text" maxlength="500" required aria-label="Task" placeholder="Add a task"><input type="datetime-local" aria-label="Remind me at"><button class="live-button" type="submit">Add</button></form><p class="live-note" role="status">Reminders appear while Tuck is running.</p><div class="todo-list"></div>';
  document.querySelector('.pages-container')!.append(page);
  const status = page.querySelector<HTMLElement>('[role=status]')!;
  const list = page.querySelector('.todo-list')!;
  let todos: Todo[];
  try { todos = normalizeTodos(JSON.parse(localStorage.getItem(KEY) || '[]')); } catch { todos = []; }
  function save(next: Todo[]): boolean {
    try { localStorage.setItem(KEY, JSON.stringify(next)); todos = next; status.textContent = 'Reminders appear while Tuck is running.'; return true; }
    catch { status.textContent = 'Could not save tasks. Check available storage.'; return false; }
  }
  function render(): void {
    const focus = (document.activeElement as HTMLElement)?.dataset.todo;
    list.replaceChildren();
    if (!todos.length) { const empty = document.createElement('p'); empty.className = 'live-note'; empty.textContent = 'No tasks yet.'; list.append(empty); }
    for (const todo of todos) {
      const row = document.createElement('div'); row.className = 'todo-row';
      const check = document.createElement('input'); check.type = 'checkbox'; check.checked = todo.done; check.dataset.todo = todo.id; check.setAttribute('aria-label', `Complete ${todo.text}`);
      check.onchange = () => { if (save(todos.map(item => item.id === todo.id ? { ...item, done: check.checked } : item))) render(); else check.checked = todo.done; };
      const label = document.createElement('span'); label.textContent = todo.text; if (todo.done) label.style.textDecoration = 'line-through';
      if (todo.dueAt) label.title = `Reminder: ${new Date(todo.dueAt).toLocaleString()}`;
      const remove = document.createElement('button'); remove.type = 'button'; remove.className = 'live-button'; remove.textContent = 'Remove';
      remove.onclick = () => { if (save(todos.filter(item => item.id !== todo.id))) render(); };
      row.append(check, label, remove); list.append(row);
    }
    if (focus) [...list.querySelectorAll<HTMLInputElement>('[data-todo]')].find(el => el.dataset.todo === focus)?.focus();
  }
  page.querySelector('form')!.onsubmit = event => {
    event.preventDefault(); const text = page.querySelector<HTMLInputElement>('input[type=text]')!;
    const date = page.querySelector<HTMLInputElement>('input[type=datetime-local]')!;
    if (!text.value.trim()) return;
    if (todos.length >= 100) { status.textContent = 'Remove a task before adding more (100 maximum).'; return; }
    const dueAt = date.value ? new Date(date.value).getTime() : null;
    if (dueAt !== null && (!Number.isFinite(dueAt) || dueAt <= Date.now())) { status.textContent = 'Choose a reminder time in the future.'; return; }
    if (save([...todos, { id: newRequestId(), text: text.value.trim(), done: false, dueAt, reminded: false }])) { text.value = ''; date.value = ''; render(); }
  };
  function checkDue(): void {
    const due = dueTodos(todos, Date.now()); if (!due.length) return;
    const ids = new Set(due.map(todo => todo.id));
    // Commit deduplication before announcing to avoid endless reminder loops on storage failure.
    if (!save(todos.map(todo => ids.has(todo.id) ? { ...todo, reminded: true } : todo))) return;
    for (const todo of due) window.dispatchEvent(new CustomEvent('tuck-preview', { detail: { title: 'Reminder', text: todo.text } }));
  }
  const interval = window.setInterval(checkDue, 1000);
  window.addEventListener('pagehide', () => clearInterval(interval), { once: true });
  window.addEventListener('storage', event => { if (event.key === KEY) { try { todos = normalizeTodos(JSON.parse(event.newValue || '[]')); render(); } catch { /* keep last valid state */ } } });
  render();
}
