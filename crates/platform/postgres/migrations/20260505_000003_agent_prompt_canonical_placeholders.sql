update llm_prompt_templates
set template_text = replace(template_text, '{{name_zh}}', '{{name}}')
where template_text like '%{{name_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{name_en}}', '{{name}}')
where template_text like '%{{name_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{marital_status_zh}}', '{{marital_status}}')
where template_text like '%{{marital_status_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{marital_status_en}}', '{{marital_status}}')
where template_text like '%{{marital_status_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{occupation_zh}}', '{{occupation}}')
where template_text like '%{{occupation_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{occupation_en}}', '{{occupation}}')
where template_text like '%{{occupation_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{personality_zh}}', '{{personality_traits}}')
where template_text like '%{{personality_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{personality_en}}', '{{personality_traits}}')
where template_text like '%{{personality_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{personality}}', '{{personality_traits}}')
where template_text like '%{{personality}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{description_zh}}', '{{persona}}')
where template_text like '%{{description_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{description_en}}', '{{persona}}')
where template_text like '%{{description_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{description}}', '{{persona}}')
where template_text like '%{{description}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{interests_zh}}', '{{private_interests}}')
where template_text like '%{{interests_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{interests_en}}', '{{private_interests}}')
where template_text like '%{{interests_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{interests}}', '{{private_interests}}')
where template_text like '%{{interests}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{traits_zh}}', '{{speaking_style}}')
where template_text like '%{{traits_zh}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{traits_en}}', '{{speaking_style}}')
where template_text like '%{{traits_en}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{traits}}', '{{speaking_style}}')
where template_text like '%{{traits}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{sexual_preference}}', '{{role_orientation}}')
where template_text like '%{{sexual_preference}}%';

update llm_prompt_templates
set template_text = replace(template_text, '{{user_sexual_preference}}', '{{role_orientation}}')
where template_text like '%{{user_sexual_preference}}%';
