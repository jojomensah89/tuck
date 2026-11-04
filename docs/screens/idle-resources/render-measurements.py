"""Render the committed, controlled Windows footprint samples (requires matplotlib)."""
import json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

root = Path(__file__).parent
names = ['before-1', 'before-2', 'after-1', 'after-2']
labels = ['Before 1', 'Before 2', 'After 1', 'After 2']
data = [json.loads((root / f'{name}.json').read_text(encoding='utf-8-sig'))['Summary'] for name in names]
rows = [[label, f"{d['DurationSeconds']:.0f}", str(d['Samples']), f"{d['WorkingSetMB']:.1f}", f"{d['PrivateMB']:.1f}", f"{d['CpuPctOfOneCore']:.2f}", f"{d['ContextSwitchesPerSecond']:.0f}"] for label, d in zip(labels, data)]
fig, ax = plt.subplots(figsize=(12, 3.2))
ax.axis('off')
ax.set_title('Tuck: collapsed, media paused, Settings closed', loc='left', pad=22, fontsize=15)
table = ax.table(cellText=rows, colLabels=['Run', 'Seconds', 'Samples', 'Working set\nMiB (upper bound)', 'Private bytes\nMiB', 'CPU %\none core', 'Context switches\nper second'], loc='center', cellLoc='center')
table.auto_set_font_size(False)
table.set_fontsize(10)
table.scale(1, 2.1)
for (r, c), cell in table.get_celld().items():
    cell.set_edgecolor('#d9dfe7')
    if r == 0:
        cell.set_facecolor('#172b40'); cell.set_text_props(color='white')
fig.text(.02, .02, 'Medians of whole process-family samples. Same machine and isolated profile; results are noisy. 2026-11-04.', fontsize=9)
fig.tight_layout()
fig.savefig(root / 'measurement-table.png', dpi=160, bbox_inches='tight')
plt.close(fig)
fig, axes = plt.subplots(2, 2, figsize=(11, 7))
metrics = [('CpuPctOfOneCore', 'CPU (% of one core)'), ('ContextSwitchesPerSecond', 'Context switches / second'), ('PrivateMB', 'Private bytes (MiB)'), ('WorkingSetMB', 'Working set (MiB, upper bound)')]
for ax, (key, title) in zip(axes.flat, metrics):
    bars = ax.bar(labels, [d[key] for d in data], color=['#305f83', '#305f83', '#bf652d', '#bf652d'])
    ax.set_title(title, loc='left'); ax.spines[['top', 'right']].set_visible(False)
    ax.bar_label(bars, fmt='%.2f' if key == 'CpuPctOfOneCore' else '%.1f', padding=4)
    ax.margins(y=.22)
fig.suptitle('Controlled idle runs: repeated medians, not a general savings guarantee', fontsize=13)
fig.tight_layout()
fig.savefig(root / 'measurement-chart.png', dpi=160)
plt.close(fig)
print('\n'.join('| ' + ' | '.join(row) + ' |' for row in rows))
