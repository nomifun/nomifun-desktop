/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import type { CharacterProps } from './types';

const Mochi: React.FC<CharacterProps> = ({ mood, activity, size = 150 }) => {
  // mood: 'happy' | 'content' | 'sleepy' | 'worried' | 'excited'
  // activity: 'idle' | 'thinking'
  const isSleepy = mood === 'sleepy';
  const isWorried = mood === 'worried';
  const isExcited = mood === 'excited';
  const isHappy = mood === 'happy';
  const isThinking = activity === 'thinking';

  return (
    <div className={`nomi-ch nomi-mochi nomi-mochi--${mood} nomi-mochi--${activity}`} style={{ width: size, height: size }}>
      <style>{`
.nomi-mochi { position: relative; display: inline-block; }
.nomi-mochi svg { display: block; overflow: visible; }
.nomi-mochi * { transform-box: view-box; }

/* ground shadow — counter-scales against body lift */
.nomi-mochi__shadow { transform-origin: 80px 147px; animation: nomi-mochi-shadow 3.4s ease-in-out infinite; }
@keyframes nomi-mochi-shadow { 0%,100%{ transform: scale(1); opacity:.13 } 50%{ transform: scale(.9); opacity:.09 } }

/* whole-body mochi squash & breathe */
.nomi-mochi__body-g { transform-origin: 80px 138px; animation: nomi-mochi-breathe 3.4s ease-in-out infinite; }
@keyframes nomi-mochi-breathe {
  0%,100%{ transform: translateY(0) scaleY(1) scaleX(1) }
  50%{ transform: translateY(-2px) scaleY(1.035) scaleX(.975) }
}

/* ears — soft alternate sway */
.nomi-mochi__ear-l { transform-origin: 64px 44px; animation: nomi-mochi-earL 3.6s ease-in-out infinite; }
.nomi-mochi__ear-r { transform-origin: 96px 44px; animation: nomi-mochi-earR 3.6s ease-in-out infinite; }
@keyframes nomi-mochi-earL { 0%,100%{ transform: rotate(2deg) } 50%{ transform: rotate(-5deg) } }
@keyframes nomi-mochi-earR { 0%,100%{ transform: rotate(-2deg) } 50%{ transform: rotate(5deg) } }

/* blink — periodic squash */
.nomi-mochi__eyes { transform-origin: 80px 92px; animation: nomi-mochi-blink 5s ease-in-out infinite; }
@keyframes nomi-mochi-blink { 0%,92%,100%{ transform: scaleY(1) } 95%{ transform: scaleY(.08) } }
.nomi-mochi--sleepy .nomi-mochi__eyes { animation: none; }

/* mouth chew while thinking */
.nomi-mochi__mouth { transform-origin: 80px 104px; }
.nomi-mochi--thinking .nomi-mochi__mouth { animation: nomi-mochi-chew .55s ease-in-out infinite; }
@keyframes nomi-mochi-chew { 0%,100%{ transform: scaleY(1) translateY(0) } 50%{ transform: scaleY(.6) translateY(1px) } }

/* happy / excited bounce overrides */
.nomi-mochi--happy .nomi-mochi__body-g { animation: nomi-mochi-hop 1.5s ease-in-out infinite; }
@keyframes nomi-mochi-hop {
  0%,100%{ transform: translateY(0) scaleY(1) } 30%{ transform: translateY(-6px) scaleY(1.04) }
  55%{ transform: translateY(0) scaleY(.94) } 70%{ transform: translateY(-1px) scaleY(1.01) }
}
.nomi-mochi--excited .nomi-mochi__body-g { animation: nomi-mochi-jump 1s ease-in-out infinite; }
@keyframes nomi-mochi-jump {
  0%,100%{ transform: translateY(0) scaleY(.95) scaleX(1.04) } 40%{ transform: translateY(-12px) scaleY(1.08) scaleX(.94) }
  60%{ transform: translateY(-10px) scaleY(1.05) } 80%{ transform: translateY(2px) scaleY(.9) scaleX(1.06) }
}
.nomi-mochi--sleepy .nomi-mochi__body-g { animation: nomi-mochi-doze 4s ease-in-out infinite; }
@keyframes nomi-mochi-doze { 0%,100%{ transform: translateY(2px) scaleY(.985) } 50%{ transform: translateY(4px) scaleY(.965) } }

/* sleepy Z's */
.nomi-mochi__z { opacity: 0; }
.nomi-mochi--sleepy .nomi-mochi__z1 { animation: nomi-mochi-z 3s ease-in-out infinite; }
.nomi-mochi--sleepy .nomi-mochi__z2 { animation: nomi-mochi-z 3s ease-in-out infinite 1.5s; }
@keyframes nomi-mochi-z {
  0%{ opacity:0; transform: translate(0,0) scale(.6) } 25%{ opacity:.9 }
  70%{ opacity:.7 } 100%{ opacity:0; transform: translate(7px,-22px) scale(1.1) }
}

/* worried sweat drop */
.nomi-mochi__sweat { opacity: 0; }
.nomi-mochi--worried .nomi-mochi__sweat { animation: nomi-mochi-sweat 2.8s ease-in-out infinite; }
@keyframes nomi-mochi-sweat {
  0%,12%{ opacity:0; transform: translateY(0) }
  22%{ opacity:.85 } 70%{ opacity:.85 } 100%{ opacity:0; transform: translateY(16px) }
}

/* excited sparkle particles */
.nomi-mochi__spark { opacity: 0; transform-origin: center; }
.nomi-mochi--excited .nomi-mochi__spark1 { animation: nomi-mochi-spark 1.1s ease-in-out infinite; }
.nomi-mochi--excited .nomi-mochi__spark2 { animation: nomi-mochi-spark 1.1s ease-in-out infinite .4s; }
.nomi-mochi--excited .nomi-mochi__spark3 { animation: nomi-mochi-spark 1.1s ease-in-out infinite .75s; }
@keyframes nomi-mochi-spark { 0%,100%{ opacity:0; transform: scale(.4) } 45%{ opacity:1; transform: scale(1.1) } }

/* thinking bubbles — small to large, serial float */
.nomi-mochi__tb { opacity: 0; }
.nomi-mochi--thinking .nomi-mochi__tb1 { animation: nomi-mochi-tb 2.4s ease-in-out infinite; }
.nomi-mochi--thinking .nomi-mochi__tb2 { animation: nomi-mochi-tb 2.4s ease-in-out infinite .35s; }
.nomi-mochi--thinking .nomi-mochi__tb3 { animation: nomi-mochi-tb 2.4s ease-in-out infinite .7s; }
@keyframes nomi-mochi-tb {
  0%{ opacity:0; transform: translateY(4px) scale(.5) } 30%{ opacity:.95 }
  75%{ opacity:.8 } 100%{ opacity:0; transform: translateY(-9px) scale(1) }
}
      `}</style>
      <svg viewBox='0 0 160 160' width={size} height={size}>
        <defs>
          <radialGradient id='mochiBody' cx='42%' cy='34%' r='72%'>
            <stop offset='0%' stopColor='#fffdfb' />
            <stop offset='55%' stopColor='#fff9f4' />
            <stop offset='100%' stopColor='#ffeede' />
          </radialGradient>
          <linearGradient id='mochiEarIn' x1='0' y1='0' x2='0' y2='1'>
            <stop offset='0%' stopColor='#ffd8e2' />
            <stop offset='100%' stopColor='#ffc0d0' />
          </linearGradient>
          <radialGradient id='mochiBlush' cx='50%' cy='50%' r='50%'>
            <stop offset='0%' stopColor='#ffb6c9' stopOpacity='0.85' />
            <stop offset='100%' stopColor='#ffc9d6' stopOpacity='0' />
          </radialGradient>
          <radialGradient id='mochiBubble' cx='38%' cy='32%' r='70%'>
            <stop offset='0%' stopColor='#fff2f6' />
            <stop offset='100%' stopColor='#ffcad9' />
          </radialGradient>
        </defs>

        {/* ground shadow */}
        <ellipse className='nomi-mochi__shadow' cx='80' cy='147' rx='38' ry='8' fill='#000000' opacity='0.12' />

        <g className='nomi-mochi__body-g'>
          {/* ===== EARS (behind body) ===== */}
          <g className='nomi-mochi__ear-l'>
            <path d='M62 60 C50 50 48 26 54 14 C58 6 68 8 70 20 C72 34 71 50 68 60 Z' fill='url(#mochiBody)' stroke='#dba6aa' strokeWidth='2.3' strokeLinejoin='round' />
            <path d='M62 54 C55 46 54 28 58 18 C61 12 66 15 66 24 C66 36 65 48 63 55 Z' fill='url(#mochiEarIn)' />
          </g>
          <g className='nomi-mochi__ear-r'>
            <path d='M98 60 C110 50 112 26 106 14 C102 6 92 8 90 20 C88 34 89 50 92 60 Z' fill='url(#mochiBody)' stroke='#dba6aa' strokeWidth='2.3' strokeLinejoin='round' />
            <path d='M98 54 C105 46 106 28 102 18 C99 12 94 15 94 24 C94 36 95 48 97 55 Z' fill='url(#mochiEarIn)' />
          </g>

          {/* ===== little paws ===== */}
          <ellipse cx='64' cy='139' rx='11' ry='8' fill='url(#mochiBody)' stroke='#dba6aa' strokeWidth='2.2' strokeLinejoin='round' />
          <ellipse cx='96' cy='139' rx='11' ry='8' fill='url(#mochiBody)' stroke='#dba6aa' strokeWidth='2.2' strokeLinejoin='round' />

          {/* ===== soft round mochi body ===== */}
          <path d='M80 50 C116 50 132 76 132 102 C132 130 110 142 80 142 C50 142 28 130 28 102 C28 76 44 50 80 50 Z' fill='url(#mochiBody)' stroke='#dba6aa' strokeWidth='2.4' strokeLinejoin='round' />

          {/* body bottom shading */}
          <path d='M34 110 C46 134 70 140 80 140 C90 140 114 134 126 110 C120 132 102 141 80 141 C58 141 40 132 34 110 Z' fill='#ffe6d2' opacity='0.55' />

          {/* top body highlight */}
          <ellipse cx='62' cy='72' rx='20' ry='13' fill='#ffffff' opacity='0.55' />
          <ellipse cx='104' cy='66' rx='7' ry='5' fill='#ffffff' opacity='0.6' />

          {/* ===== blush ===== */}
          <ellipse cx='52' cy='102' rx='11' ry='7' fill='url(#mochiBlush)' />
          <ellipse cx='108' cy='102' rx='11' ry='7' fill='url(#mochiBlush)' />

          {/* ===== EYES ===== */}
          <g className='nomi-mochi__eyes'>
            {isSleepy ? (
              <>
                <path d='M56 92 Q63 99 70 92' fill='none' stroke='#7a5a52' strokeWidth='3' strokeLinecap='round' />
                <path d='M90 92 Q97 99 104 92' fill='none' stroke='#7a5a52' strokeWidth='3' strokeLinecap='round' />
              </>
            ) : isHappy ? (
              <>
                <path d='M55 95 Q63 86 71 95' fill='none' stroke='#5a3d38' strokeWidth='3.4' strokeLinecap='round' />
                <path d='M89 95 Q97 86 105 95' fill='none' stroke='#5a3d38' strokeWidth='3.4' strokeLinecap='round' />
              </>
            ) : isExcited ? (
              <>
                <circle cx='63' cy='92' r='8' fill='#4a322d' />
                <circle cx='97' cy='92' r='8' fill='#4a322d' />
                <circle cx='60' cy='89' r='2.8' fill='#fff' />
                <circle cx='94' cy='89' r='2.8' fill='#fff' />
                <circle cx='65' cy='95' r='1.6' fill='#fff' opacity='0.8' />
                <circle cx='99' cy='95' r='1.6' fill='#fff' opacity='0.8' />
              </>
            ) : (
              <>
                <ellipse cx='63' cy='92' rx='6' ry='7.5' fill='#4a322d' />
                <ellipse cx='97' cy='92' rx='6' ry='7.5' fill='#4a322d' />
                <circle cx='61' cy='89' r='2.4' fill='#fff' />
                <circle cx='95' cy='89' r='2.4' fill='#fff' />
                <circle cx='65' cy='94' r='1.2' fill='#fff' opacity='0.7' />
                <circle cx='99' cy='94' r='1.2' fill='#fff' opacity='0.7' />
              </>
            )}
          </g>

          {/* worried brows */}
          {isWorried && (
            <>
              <path d='M55 82 Q63 80 70 84' fill='none' stroke='#c79a93' strokeWidth='2.4' strokeLinecap='round' />
              <path d='M90 84 Q97 80 105 82' fill='none' stroke='#c79a93' strokeWidth='2.4' strokeLinecap='round' />
            </>
          )}

          {/* ===== nose + mouth ===== */}
          <ellipse cx='80' cy='100' rx='3' ry='2.2' fill='#e89aa8' />
          <g className='nomi-mochi__mouth'>
            {isSleepy ? (
              <path d='M74 106 Q80 109 86 106' fill='none' stroke='#a86a64' strokeWidth='2.2' strokeLinecap='round' />
            ) : isWorried ? (
              <path d='M73 108 Q80 102 87 108' fill='none' stroke='#a86a64' strokeWidth='2.4' strokeLinecap='round' />
            ) : isExcited ? (
              <path d='M72 104 Q80 114 88 104 Q80 109 72 104 Z' fill='#e98a98' stroke='#a86a64' strokeWidth='2' strokeLinejoin='round' />
            ) : (
              <path d='M74 104 Q80 110 86 104' fill='none' stroke='#a86a64' strokeWidth='2.4' strokeLinecap='round' />
            )}
          </g>

          {/* worried sweat */}
          <g className='nomi-mochi__sweat'>
            <path d='M114 80 C114 80 109 87 109 91 a5 5 0 0 0 10 0 C119 87 114 80 114 80 Z' fill='#9fd9ef' stroke='#7cc4e0' strokeWidth='1' />
            <ellipse cx='112' cy='88' rx='1.5' ry='2' fill='#fff' opacity='0.7' />
          </g>
        </g>

        {/* ===== sleepy Z's (head-top) ===== */}
        {isSleepy && (
          <g fill='#b79be0' fontFamily='sans-serif' fontWeight='700'>
            <text className='nomi-mochi__z nomi-mochi__z1' x='104' y='48' fontSize='13'>z</text>
            <text className='nomi-mochi__z nomi-mochi__z2' x='112' y='38' fontSize='17'>Z</text>
          </g>
        )}

        {/* ===== excited sparkles ===== */}
        {isExcited && (
          <g fill='#ffd84d'>
            <path className='nomi-mochi__spark nomi-mochi__spark1' d='M118 60 l2 5 5 2 -5 2 -2 5 -2 -5 -5 -2 5 -2 Z' />
            <path className='nomi-mochi__spark nomi-mochi__spark2' d='M40 56 l1.6 4 4 1.6 -4 1.6 -1.6 4 -1.6 -4 -4 -1.6 4 -1.6 Z' />
            <path className='nomi-mochi__spark nomi-mochi__spark3' d='M128 88 l1.4 3.5 3.5 1.4 -3.5 1.4 -1.4 3.5 -1.4 -3.5 -3.5 -1.4 3.5 -1.4 Z' />
          </g>
        )}

        {/* ===== thinking bubbles ===== */}
        {isThinking && (
          <g>
            <circle className='nomi-mochi__tb nomi-mochi__tb1' cx='112' cy='52' r='3' fill='url(#mochiBubble)' stroke='#f3b9cb' strokeWidth='0.8' />
            <circle className='nomi-mochi__tb nomi-mochi__tb2' cx='120' cy='42' r='4.5' fill='url(#mochiBubble)' stroke='#f3b9cb' strokeWidth='0.8' />
            <circle className='nomi-mochi__tb nomi-mochi__tb3' cx='130' cy='30' r='6.5' fill='url(#mochiBubble)' stroke='#f3b9cb' strokeWidth='0.9' />
          </g>
        )}
      </svg>
    </div>
  );
};

export default Mochi;
