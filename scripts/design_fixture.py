"""Create isolated, offline review data. Never reads or writes the user's account."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
english = '--english' in sys.argv[2:]
site = 'global' if english else 'china'
root.mkdir(parents=True, exist_ok=True)
base = Path(__file__).resolve().parents[1]
details = json.loads((base / 'assets/lesson-details.json').read_text(encoding='utf-8'))
problems = []
for number, slug, title in [
    (994, 'rotting-oranges', '腐烂的橘子'),
    (127, 'word-ladder', '单词接龙'),
    (752, 'open-the-lock', '打开转盘锁'),
    (1091, 'shortest-path-in-binary-matrix', '二进制矩阵中的最短路径'),
]:
    if english:
        title = {994:'Rotting Oranges',127:'Word Ladder',752:'Open the Lock',1091:'Shortest Path in Binary Matrix'}[number]
    problems.append(dict(
        id=number, slug=slug, title=title, difficulty='Medium', tags=['广度优先搜索'],
        description='<p>网格中的 0 表示空格，1 表示新鲜橘子，2 表示腐烂的橘子。每分钟，腐烂橘子会感染上下左右相邻的新鲜橘子。返回所有橘子腐烂所需的最少分钟数；无法全部感染时返回 -1。</p><pre>grid = [[2,1,1],[1,1,0],[0,1,1]]\n输出：4</pre>',
        starter='package solution\n\n' + details[slug]['go'], function='orangesRotting',
        parameters=[], returns='int', cases=[], explanation='',
        online=dict(site=site, frontend_id=str(number), question_id=str(number),
                    status=None, paid_only=False, loaded=True, go_supported=True,
                    java_supported=False, sample_input='[[2,1,1],[1,1,0],[0,1,1]]'),
    ))
cache=root/('leetcode/global' if english else 'leetcode/cn')
cache.mkdir(parents=True,exist_ok=True)
(cache/'index.json').write_text(json.dumps(dict(site=site,username='design-fixture',problems=problems,synced_at=1),ensure_ascii=False),encoding='utf-8')
for name,value in [('appearance','beige'),('site',site),('workspace-layout','study'),('session',994)]:
    (root/f'{name}.json').write_text(json.dumps(value),encoding='utf-8')
print(root)
