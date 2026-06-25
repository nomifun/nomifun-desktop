/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import type { CharacterProps } from './types';

const Ink: React.FC<CharacterProps> = ({ mood, activity, size = 150 }) => {
  // mood: 'happy' | 'content' | 'sleepy' | 'worried' | 'excited'
  // activity: 'idle' | 'thinking'
  const sleeping = mood === 'sleepy';
  return (
    <div className={`nomi-ch nomi-ink nomi-ink--${mood} nomi-ink--${activity}`} style={{ width: size, height: size }}>
      <style>{`
.nomi-ink{position:relative;line-height:0}
.nomi-ink svg{display:block;overflow:visible}
.nomi-ink *{transform-box:view-box}

/* ground shadow — inverse-scales with the body lift */
.nomi-ink__shadow{transform-origin:80px 147px;animation:nomi-ink-shadow 3.4s ease-in-out infinite}
@keyframes nomi-ink-shadow{0%,100%{transform:scale(1);opacity:.13}50%{transform:scale(.9);opacity:.09}}

/* whole body: calm breathing + gentle lift */
.nomi-ink__body-g{transform-origin:80px 142px;animation:nomi-ink-breathe 3.4s ease-in-out infinite}
@keyframes nomi-ink-breathe{0%,100%{transform:translateY(0) scaleY(1)}50%{transform:translateY(-1.5px) scaleY(1.03)}}

/* tail — elegant slow sway from the root */
.nomi-ink__tail{transform-origin:104px 134px;animation:nomi-ink-tail 4.6s ease-in-out infinite}
@keyframes nomi-ink-tail{0%,100%{transform:rotate(-2.5deg)}50%{transform:rotate(2.5deg)}}

/* ears — occasional quick flick */
.nomi-ink__ear-l{transform-origin:60px 44px;animation:nomi-ink-ear 5.2s ease-in-out infinite}
.nomi-ink__ear-r{transform-origin:100px 44px;animation:nomi-ink-ear 5.2s ease-in-out infinite .4s}
@keyframes nomi-ink-ear{0%,86%,100%{transform:rotate(0)}90%{transform:rotate(-7deg)}94%{transform:rotate(4deg)}}

/* auto blink */
.nomi-ink__eyes{transform-origin:80px 80px;animation:nomi-ink-blink 5s ease-in-out infinite}
@keyframes nomi-ink-blink{0%,92%,100%{transform:scaleY(1)}96%{transform:scaleY(.08)}}

.nomi-ink__blush{opacity:0;animation:nomi-ink-blush 3.4s ease-in-out infinite}
@keyframes nomi-ink-blush{0%,100%{opacity:.32}50%{opacity:.5}}

/* === mood overrides === */
.nomi-ink--happy .nomi-ink__body-g{animation:nomi-ink-hop 1.7s ease-in-out infinite}
@keyframes nomi-ink-hop{0%,100%{transform:translateY(0)}50%{transform:translateY(-4px)}}

.nomi-ink--excited .nomi-ink__body-g{animation:nomi-ink-bounce 1s ease-in-out infinite}
@keyframes nomi-ink-bounce{0%,100%{transform:translateY(0) scaleY(1)}45%{transform:translateY(-9px) scaleY(1.05)}70%{transform:translateY(1px) scaleY(.96)}}
.nomi-ink--excited .nomi-ink__shadow{animation:nomi-ink-shadow 1s ease-in-out infinite}
.nomi-ink--excited .nomi-ink__eyes{animation:nomi-ink-blink 6s ease-in-out infinite}

.nomi-ink--sleepy .nomi-ink__body-g{animation:nomi-ink-sink 4.4s ease-in-out infinite}
@keyframes nomi-ink-sink{0%,100%{transform:translateY(2px) scaleY(.99)}50%{transform:translateY(3.5px) scaleY(1.01)}}
.nomi-ink--sleepy .nomi-ink__tail{animation:none;transform:rotate(0)}
.nomi-ink--sleepy .nomi-ink__eyes{animation:none}

/* z's drift up serially */
.nomi-ink__z{opacity:0}
.nomi-ink__z1{animation:nomi-ink-z 3.2s ease-in-out infinite}
.nomi-ink__z2{animation:nomi-ink-z 3.2s ease-in-out infinite 1.6s}
@keyframes nomi-ink-z{0%{opacity:0;transform:translate(0,0) scale(.7)}25%{opacity:.85}100%{opacity:0;transform:translate(7px,-20px) scale(1.1)}}

/* worried sweat drop slides + ears droop */
.nomi-ink__sweat{opacity:0;animation:nomi-ink-sweat 2.8s ease-in-out infinite}
@keyframes nomi-ink-sweat{0%,18%{opacity:0;transform:translateY(0)}30%{opacity:.85}70%{opacity:.8}100%{opacity:0;transform:translateY(15px)}}
.nomi-ink--worried .nomi-ink__ear-l{animation:none;transform:rotate(11deg)}
.nomi-ink--worried .nomi-ink__ear-r{animation:none;transform:rotate(-11deg)}

/* excited sparkles */
.nomi-ink__spark{opacity:0;transform-origin:center}
.nomi-ink__spark1{animation:nomi-ink-spark 1.3s ease-in-out infinite}
.nomi-ink__spark2{animation:nomi-ink-spark 1.3s ease-in-out infinite .45s}
.nomi-ink__spark3{animation:nomi-ink-spark 1.3s ease-in-out infinite .85s}
@keyframes nomi-ink-spark{0%,100%{opacity:0;transform:scale(.3)}50%{opacity:1;transform:scale(1)}}

/* thinking: ink droplets rise and bloom away */
.nomi-ink__drop{opacity:0}
.nomi-ink--thinking .nomi-ink__drop1{animation:nomi-ink-drop 2.6s ease-in-out infinite}
.nomi-ink--thinking .nomi-ink__drop2{animation:nomi-ink-drop 2.6s ease-in-out infinite .85s}
.nomi-ink--thinking .nomi-ink__drop3{animation:nomi-ink-drop 2.6s ease-in-out infinite 1.7s}
@keyframes nomi-ink-drop{0%{opacity:0;transform:translateY(0) scale(.5)}20%{opacity:.9;transform:translateY(-4px) scale(1)}70%{opacity:.55;transform:translateY(-16px) scale(1.15)}100%{opacity:0;transform:translateY(-24px) scale(2.1)}}

@media(prefers-reduced-motion:reduce){.nomi-ink *{animation-duration:6s!important}}
      `}</style>
      <svg viewBox='0 0 160 160' width={size} height={size}>
        <defs>
          <radialGradient id='inkBody' cx='42%' cy='30%' r='78%'>
            <stop offset='0%' stopColor='#52525f' />
            <stop offset='42%' stopColor='#3a3a44' />
            <stop offset='100%' stopColor='#1f1f26' />
          </radialGradient>
          <linearGradient id='inkTail' x1='0' y1='0' x2='1' y2='1'>
            <stop offset='0%' stopColor='#3c3c46' />
            <stop offset='100%' stopColor='#222229' />
          </linearGradient>
          <radialGradient id='inkEye' cx='42%' cy='34%' r='72%'>
            <stop offset='0%' stopColor='#ffe39a' />
            <stop offset='48%' stopColor='#f0b346' />
            <stop offset='100%' stopColor='#d4902a' />
          </radialGradient>
          <radialGradient id='inkBlush' cx='50%' cy='50%' r='50%'>
            <stop offset='0%' stopColor='#e9849a' stopOpacity='.85' />
            <stop offset='100%' stopColor='#e9849a' stopOpacity='0' />
          </radialGradient>
          <radialGradient id='inkEarIn' cx='50%' cy='40%' r='65%'>
            <stop offset='0%' stopColor='#4a4a58' />
            <stop offset='100%' stopColor='#2a2a32' />
          </radialGradient>
        </defs>

        {/* contact shadow */}
        <ellipse className='nomi-ink__shadow' cx='80' cy='147' rx='34' ry='7' fill='#000' opacity='.13' />

        <g className='nomi-ink__body-g'>
          {/* tail — wraps around the front paws, white tip resting in front */}
          <g className='nomi-ink__tail'>
            <path
              d='M104 134 C122 132 128 122 124 112 C120 124 108 130 92 132 C70 142 48 142 42 134 C44 144 70 148 92 142'
              fill='none'
              stroke='url(#inkTail)'
              strokeWidth='12'
              strokeLinecap='round'
            />
            {/* white tail tip */}
            <circle cx='44' cy='137' r='6.5' fill='#f4f1ea' />
            <circle cx='42.5' cy='135' r='2.4' fill='#fff' opacity='.7' />
          </g>

          {/* seated body — slightly irregular brush silhouette */}
          <path
            d='M80 60 C58 60 47 78 46 100 C45 116 49 132 56 138 C64 145 96 145 104 138 C111 132 115 116 114 100 C113 78 102 60 80 60 Z'
            fill='url(#inkBody)'
            stroke='#15151b'
            strokeWidth='2.4'
            strokeLinejoin='round'
          />
          {/* rim light — inner edge glow for black-on-dark readability */}
          <path
            d='M80 63 C60 63 50 80 49 100 C48 114 52 129 58 135'
            fill='none'
            stroke='#5a5a6e'
            strokeWidth='2.2'
            strokeLinecap='round'
            opacity='.6'
          />
          {/* crescent-moon white chest mark */}
          <path d='M73 116 C73 126 87 126 87 116 C84 122 76 122 73 116 Z' fill='#f4f1ea' opacity='.92' />

          {/* head */}
          <g>
            {/* ears */}
            <g className='nomi-ink__ear-l'>
              <path d='M52 52 L58 26 L74 46 Z' fill='url(#inkBody)' stroke='#15151b' strokeWidth='2.3' strokeLinejoin='round' />
              <path d='M58 46 L61 33 L69 45 Z' fill='url(#inkEarIn)' />
            </g>
            <g className='nomi-ink__ear-r'>
              <path d='M108 52 L102 26 L86 46 Z' fill='url(#inkBody)' stroke='#15151b' strokeWidth='2.3' strokeLinejoin='round' />
              <path d='M102 46 L99 33 L91 45 Z' fill='url(#inkEarIn)' />
            </g>

            <path
              d='M80 36 C57 36 44 52 44 72 C44 92 60 102 80 102 C100 102 116 92 116 72 C116 52 103 36 80 36 Z'
              fill='url(#inkBody)'
              stroke='#15151b'
              strokeWidth='2.4'
              strokeLinejoin='round'
            />
            {/* head rim light */}
            <path d='M80 39 C60 39 47 53 47 71' fill='none' stroke='#5a5a6e' strokeWidth='2.2' strokeLinecap='round' opacity='.55' />
            {/* top crown highlight */}
            <ellipse cx='72' cy='50' rx='15' ry='8' fill='#fff' opacity='.08' />

            {/* blush */}
            <g className='nomi-ink__blush'>
              <ellipse cx='56' cy='80' rx='8' ry='5' fill='url(#inkBlush)' />
              <ellipse cx='104' cy='80' rx='8' ry='5' fill='url(#inkBlush)' />
            </g>

            {/* eyes */}
            <g className='nomi-ink__eyes'>
              {sleeping ? (
                <>
                  <path d='M55 74 C60 80 70 80 75 74' fill='none' stroke='#f0b346' strokeWidth='3' strokeLinecap='round' />
                  <path d='M85 74 C90 80 100 80 105 74' fill='none' stroke='#f0b346' strokeWidth='3' strokeLinecap='round' />
                </>
              ) : mood === 'happy' ? (
                <>
                  <path d='M54 76 C60 68 70 68 76 76' fill='none' stroke='#f0b346' strokeWidth='4.5' strokeLinecap='round' />
                  <path d='M84 76 C90 68 100 68 106 76' fill='none' stroke='#f0b346' strokeWidth='4.5' strokeLinecap='round' />
                </>
              ) : mood === 'worried' ? (
                <>
                  {/* worried droop brows + smaller eyes */}
                  <path d='M53 67 L73 71' fill='none' stroke='#15151b' strokeWidth='2.4' strokeLinecap='round' />
                  <path d='M107 67 L87 71' fill='none' stroke='#15151b' strokeWidth='2.4' strokeLinecap='round' />
                  <ellipse cx='65' cy='78' rx='6.5' ry='8.5' fill='url(#inkEye)' />
                  <ellipse cx='95' cy='78' rx='6.5' ry='8.5' fill='url(#inkEye)' />
                  <ellipse cx='65' cy='79' rx='2.3' ry='6.5' fill='#1a1208' />
                  <ellipse cx='95' cy='79' rx='2.3' ry='6.5' fill='#1a1208' />
                  <circle cx='63' cy='75' r='1.9' fill='#fff' />
                  <circle cx='93' cy='75' r='1.9' fill='#fff' />
                </>
              ) : mood === 'excited' ? (
                <>
                  {/* dilated round eyes with double highlight */}
                  <ellipse cx='65' cy='78' rx='9.5' ry='11' fill='url(#inkEye)' />
                  <ellipse cx='95' cy='78' rx='9.5' ry='11' fill='url(#inkEye)' />
                  <ellipse cx='65' cy='79' rx='5' ry='8.5' fill='#1a1208' />
                  <ellipse cx='95' cy='79' rx='5' ry='8.5' fill='#1a1208' />
                  <circle cx='62' cy='74' r='3' fill='#fff' />
                  <circle cx='92' cy='74' r='3' fill='#fff' />
                  <circle cx='68' cy='82' r='1.6' fill='#fff' opacity='.85' />
                  <circle cx='98' cy='82' r='1.6' fill='#fff' opacity='.85' />
                </>
              ) : (
                <>
                  {/* default amber vertical-slit eyes */}
                  <ellipse cx='65' cy='78' rx='8' ry='10.5' fill='url(#inkEye)' />
                  <ellipse cx='95' cy='78' rx='8' ry='10.5' fill='url(#inkEye)' />
                  <ellipse cx='65' cy='79' rx='2.6' ry='8' fill='#1a1208' />
                  <ellipse cx='95' cy='79' rx='2.6' ry='8' fill='#1a1208' />
                  <circle cx='62' cy='74' r='2.2' fill='#fff' />
                  <circle cx='92' cy='74' r='2.2' fill='#fff' />
                  <circle cx='67' cy='82' r='1.3' fill='#fff' opacity='.7' />
                  <circle cx='97' cy='82' r='1.3' fill='#fff' opacity='.7' />
                </>
              )}
            </g>

            {/* nose + mouth */}
            <path d='M77 88 L83 88 L80 92 Z' fill='#e58aa0' />
            {mood === 'worried' ? (
              <path d='M72 99 C76 95 84 95 88 99' fill='none' stroke='#15151b' strokeWidth='2' strokeLinecap='round' />
            ) : mood === 'happy' || mood === 'excited' ? (
              <path d='M70 94 C74 102 86 102 90 94' fill='none' stroke='#15151b' strokeWidth='2.2' strokeLinecap='round' />
            ) : (
              <path d='M80 92 C76 97 73 96 71 94 M80 92 C84 97 87 96 89 94' fill='none' stroke='#15151b' strokeWidth='1.9' strokeLinecap='round' />
            )}

            {/* whiskers */}
            <g stroke='#15151b' strokeWidth='1.3' strokeLinecap='round' opacity='.5'>
              <path d='M50 84 L36 82' />
              <path d='M50 89 L37 90' />
              <path d='M110 84 L124 82' />
              <path d='M110 89 L123 90' />
            </g>
          </g>
        </g>

        {/* sleepy z's */}
        {sleeping && (
          <g fill='#cfcfdc' fontFamily='Georgia, serif' fontStyle='italic' fontWeight='700'>
            <text className='nomi-ink__z nomi-ink__z1' x='108' y='40' fontSize='13'>z</text>
            <text className='nomi-ink__z nomi-ink__z2' x='116' y='30' fontSize='17'>z</text>
          </g>
        )}

        {/* worried cold sweat */}
        {mood === 'worried' && (
          <path className='nomi-ink__sweat' d='M112 56 C112 52 116 50 116 50 C116 50 120 52 120 56 C120 59 118 61 116 61 C114 61 112 59 112 56 Z' fill='#9fdcf5' stroke='#6bb8df' strokeWidth='1' />
        )}

        {/* excited sparkles */}
        {mood === 'excited' && (
          <g fill='#ffe39a'>
            <path className='nomi-ink__spark nomi-ink__spark1' d='M118 44 l2 5 5 2 -5 2 -2 5 -2 -5 -5 -2 5 -2 Z' />
            <path className='nomi-ink__spark nomi-ink__spark2' d='M36 50 l1.5 4 4 1.5 -4 1.5 -1.5 4 -1.5 -4 -4 -1.5 4 -1.5 Z' />
            <path className='nomi-ink__spark nomi-ink__spark3' d='M126 70 l1.2 3 3 1.2 -3 1.2 -1.2 3 -1.2 -3 -3 -1.2 3 -1.2 Z' />
          </g>
        )}

        {/* thinking ink droplets */}
        {activity === 'thinking' && (
          <g fill='#2b2b33' opacity='.78'>
            <path className='nomi-ink__drop nomi-ink__drop1' d='M78 28 C78 24 80 20 80 20 C80 20 82 24 82 28 C82 31 80.5 33 80 33 C79.5 33 78 31 78 28 Z' />
            <path className='nomi-ink__drop nomi-ink__drop2' d='M88 30 C88 27 89.5 24 89.5 24 C89.5 24 91 27 91 30 C91 32 90 33.5 89.5 33.5 C89 33.5 88 32 88 30 Z' />
            <path className='nomi-ink__drop nomi-ink__drop3' d='M70 31 C70 28 71.5 25 71.5 25 C71.5 25 73 28 73 31 C73 33 72 34.5 71.5 34.5 C71 34.5 70 33 70 31 Z' />
          </g>
        )}
      </svg>
    </div>
  );
};

export default Ink;
