"""Refresh authored-content counts against the observed reference directories.

This records presence of original content, not proof of correctness or proof
that a reference website is exhaustively indexed. Run the Go and Rust checks
separately before recording verification results or packaging a release.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AUDIT = ROOT / 'design/curriculum-audit'


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def main():
    lessons = read(ROOT / 'assets/problem-lessons.json')
    details = read(ROOT / 'assets/lesson-details.json')
    chapters = read(ROOT / 'assets/learning-path.json')
    courses = read(ROOT / 'assets/curriculum.json')
    curated = {slug for chapter in chapters for pattern in chapter['patterns']
               for slug in pattern['questions']}
    if curated != set(lessons):
        raise SystemExit('Curated membership and primary explanations differ')
    expected_details = set(lessons)
    for slug, lesson in lessons.items():
        expected_details.update(f'{slug}@{pattern}' for pattern in lesson.get('alternatives', {}))
    if expected_details != set(details):
        raise SystemExit('Primary/alternative explanations and detailed bodies differ')
    inventory_path = AUDIT / 'exercise-inventory.json'
    inventory = read(inventory_path)
    unique = {}
    pending = {}
    for group in inventory['items']:
        for question in group['questions']:
            slug = question.get('slug')
            question['has_original_detailed_explanation'] = slug in details
            unique[str(question['number'])] = question
            if not question['has_original_detailed_explanation']:
                key = slug or f"unknown-number:{question['number']}"
                item = pending.setdefault(key, {
                    'number': question['number'], 'slug': slug, 'title': question['title'],
                    'reference_sets': [], 'status': 'needs_original_explanation_and_verified_go',
                })
                item['reference_sets'].append(group['url'])
        group['original_detailed_count'] = sum(
            q['has_original_detailed_explanation'] for q in group['questions'])
        group['missing_numbers'] = [q['number'] for q in group['questions']
                                    if not q['has_original_detailed_explanation']]
    inventory.update(
        unique_questions=len(unique),
        original_details_present=sum(q['has_original_detailed_explanation']
                                     for q in unique.values()),
        missing_original_details=len(pending),
        complete_observed_sets=sum(bool(g['questions']) and not g['missing_numbers']
                                   for g in inventory['items']),
    )
    write(inventory_path, inventory)
    exercise_question_count = len(unique)
    framework_path = AUDIT / 'framework-example-inventory.json'
    framework = read(framework_path) if framework_path.exists() else {'items': []}
    framework_unique = {}
    for group in framework['items']:
        for question in group['questions']:
            slug = question.get('slug')
            question['has_original_detailed_explanation'] = slug in details
            number = str(question['number'])
            unique[number] = question
            framework_unique[number] = question
            if not question['has_original_detailed_explanation']:
                key = slug or f'unknown-number:{number}'
                item = pending.setdefault(key, {
                    'number': question['number'], 'slug': slug,
                    'title': question.get('title', (slug or number).replace('-', ' ')),
                    'reference_sets': [], 'status': 'needs_original_explanation_and_verified_go',
                })
                item['reference_sets'].append(group['url'])
        group['original_detailed_count'] = sum(
            q['has_original_detailed_explanation'] for q in group['questions'])
        group['missing_numbers'] = [q['number'] for q in group['questions']
                                    if not q['has_original_detailed_explanation']]
    if framework_path.exists():
        framework['missing_original_details'] = [q for q in framework_unique.values()
                                                 if not q['has_original_detailed_explanation']]
        write(framework_path, framework)
    bodies_present = sum(q['has_original_detailed_explanation'] for q in unique.values())
    write(AUDIT / 'problem-content-plan.json', {
        'scope': '按当前观察题集及已核对框架例题逐题补充原创中英文解析；不是复制参考站正文。',
        'observed_question_count': len(unique),
        'observed_exercise_question_count': exercise_question_count,
        'observed_framework_example_question_count': len(framework_unique),
        'original_bodies_present_in_observed_sets': bodies_present,
        'count_semantics': '题数取题集与已核对框架例题的并集，按正文存在性统计；正确性与全站全集仍须独立验证。',
        'remaining_count': len(pending), 'remaining': list(pending.values()),
        'completion_claim': False,
    })
    coverage_path = AUDIT / 'topic-coverage.json'
    coverage = read(coverage_path)
    coverage['original_lesson_count'] = len(courses)
    coverage['original_problem_explanations'] = len(details)
    memberships = {}
    for chapter in chapters:
        for pattern in chapter['patterns']:
            for slug in pattern['questions']:
                memberships.setdefault(slug, []).append(pattern['id'])
    groups = {group['url']: group for group in inventory['items']}
    groups.update((group['url'], group) for group in framework['items'])
    for item in coverage['items']:
        group = groups.get(item['reference_url'])
        if group is None:
            continue
        item['observed_reference_question_numbers'] = [q['number'] for q in group['questions']]
        item['implemented_observed_question_slugs'] = [q['slug'] for q in group['questions']
                                                       if q['has_original_detailed_explanation']]
        item['practice_slugs'] = item['implemented_observed_question_slugs']
        item['practice_patterns'] = list(dict.fromkeys(
            pattern for slug in item['practice_slugs']
            for pattern in memberships.get(slug, [])
        ))
        item['missing_observed_question_numbers'] = group['missing_numbers']
        item['scope_note'] = ('仅按当前观察目录核对原创正文存在性。'
                              '目录清单不证明全站全集，正确性需另行验证。')
    write(coverage_path, coverage)
    print(json.dumps({
        'courses': len(courses), 'chapters': len(chapters),
        'patterns': sum(len(c['patterns']) for c in chapters),
        'original_questions': len(lessons), 'solution_variants': len(details),
        'observed_reference_questions': len(unique),
        'observed_reference_bodies_present': bodies_present,
        'remaining_exercise_questions': inventory['missing_original_details'],
        'remaining_framework_example_questions': sum(
            not q['has_original_detailed_explanation'] for q in framework_unique.values()),
        'remaining_observed_questions': len(pending),
        'complete_observed_sets': inventory['complete_observed_sets'],
    }, ensure_ascii=False))


if __name__ == '__main__':
    main()
